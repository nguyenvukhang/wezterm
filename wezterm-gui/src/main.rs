// Don't create a new standard console window when launched from the windows GUI.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use ::window::*;
use anyhow::{anyhow, Context};
use clap::builder::ValueParser;
use clap::{Parser, ValueHint};
use config::keyassignment::{SpawnCommand, SpawnTabDomain};
use config::ConfigHandle;
use mux::activity::Activity;
use mux::domain::{Domain, LocalDomain};
use mux::Mux;
use mux_lua::MuxDomain;
use portable_pty::cmdbuilder::CommandBuilder;
use promise::spawn::block_on;
use std::borrow::Cow;
use std::env::current_dir;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use wezterm_client::domain::ClientDomain;
use wezterm_gui_subcommands::*;
use wezterm_toast_notification::*;

mod colorease;
mod commands;
mod customglyph;
mod frontend;
mod glyphcache;
mod inputmap;
mod overlay;
mod quad;
mod renderstate;
mod resize_increment_calculator;
mod scripting;
mod scrollbar;
mod selection;
mod shapecache;
mod spawn;
mod tabbar;
mod termwindow;
mod uniforms;
mod utilsprites;

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

pub use selection::SelectionMode;
pub use termwindow::{set_window_class, set_window_position, TermWindow, ICON_DATA};

#[derive(Debug, Parser)]
#[command(
    about = "Wez's Terminal Emulator\nhttp://github.com/wez/wezterm",
    version = config::wezterm_version()
)]
struct Opt {
    /// Skip loading wezterm.lua
    #[arg(long, short = 'n')]
    skip_config: bool,

    /// Specify the configuration file to use, overrides the normal
    /// configuration file resolution
    #[arg(
        long = "config-file",
        value_parser,
        conflicts_with = "skip_config",
        value_hint=ValueHint::FilePath,
    )]
    config_file: Option<OsString>,

    /// Override specific configuration values
    #[arg(
        long = "config",
        name = "name=value",
        value_parser=ValueParser::new(name_equals_value),
        number_of_values = 1)]
    config_override: Vec<(String, String)>,

    /// On Windows, whether to attempt to attach to the parent
    /// process console to display logging output
    #[arg(long = "attach-parent-console")]
    #[allow(dead_code)]
    attach_parent_console: bool,

    #[command(subcommand)]
    cmd: Option<SubCommand>,
}

#[derive(Debug, Parser, Clone)]
enum SubCommand {
    #[command(
        name = "start",
        about = "Start the GUI, optionally running an alternative program"
    )]
    Start(StartCommand),
}

fn have_panes_in_domain_and_ws(domain: &Arc<dyn Domain>, workspace: &Option<String>) -> bool {
    let mux = Mux::get();
    let have_panes_in_domain = mux
        .iter_panes()
        .iter()
        .any(|p| p.domain_id() == domain.domain_id());

    if !have_panes_in_domain {
        return false;
    }

    if let Some(ws) = &workspace {
        for window_id in mux.iter_windows_in_workspace(ws) {
            if let Some(win) = mux.get_window(window_id) {
                for t in win.iter() {
                    for p in t.iter_panes_ignoring_zoom() {
                        if p.pane.domain_id() == domain.domain_id() {
                            return true;
                        }
                    }
                }
            }
        }
        false
    } else {
        true
    }
}

async fn spawn_tab_in_domain_if_mux_is_empty(
    cmd: Option<CommandBuilder>,
    is_connecting: bool,
    domain: Option<Arc<dyn Domain>>,
    workspace: Option<String>,
) -> anyhow::Result<()> {
    let mux = Mux::get();

    let domain = domain.unwrap_or_else(|| mux.default_domain());

    if !is_connecting {
        if have_panes_in_domain_and_ws(&domain, &workspace) {
            return Ok(());
        }
    }

    let window_id = {
        // Force the builder to notify the frontend early,
        // so that the attach await below doesn't block it.
        // This has the consequence of creating the window
        // at the initial size instead of populating it
        // from the size specified in the remote mux.
        // We use the TabAddedToWindow mux notification
        // to detect and adjust the size later on.
        let position = None;
        let builder = mux.new_empty_window(workspace.clone(), position);
        *builder
    };

    let config = config::configuration();
    config.update_ulimit()?;

    domain.attach(Some(window_id)).await?;

    if have_panes_in_domain_and_ws(&domain, &workspace) {
        trigger_and_log_gui_attached(MuxDomain(domain.domain_id())).await;
        return Ok(());
    }

    let dpi = config.dpi.unwrap_or_else(|| ::window::default_dpi()) as u32;
    let _tab = domain
        .spawn(config.initial_size(dpi), cmd, None, window_id)
        .await?;
    trigger_and_log_gui_attached(MuxDomain(domain.domain_id())).await;
    Ok(())
}

async fn connect_to_auto_connect_domains() -> anyhow::Result<()> {
    let mux = Mux::get();
    let domains = mux.iter_domains();
    for dom in domains {
        if let Some(dom) = dom.downcast_ref::<ClientDomain>() {
            if dom.connect_automatically() {
                dom.attach(None).await?;
            }
        }
    }
    Ok(())
}

async fn trigger_gui_startup(
    lua: Option<Rc<mlua::Lua>>,
    spawn: Option<SpawnCommand>,
) -> anyhow::Result<()> {
    if let Some(lua) = lua {
        let args = lua.pack_multi(spawn)?;
        config::lua::emit_event(&lua, ("gui-startup".to_string(), args)).await?;
    }
    Ok(())
}

async fn trigger_and_log_gui_startup(spawn_command: Option<SpawnCommand>) {
    if let Err(err) =
        config::with_lua_config_on_main_thread(move |lua| trigger_gui_startup(lua, spawn_command))
            .await
    {
        let message = format!("while processing gui-startup event: {:#}", err);
        log::error!("{}", message);
        persistent_toast_notification("Error", &message);
    }
}

async fn trigger_gui_attached(lua: Option<Rc<mlua::Lua>>, domain: MuxDomain) -> anyhow::Result<()> {
    if let Some(lua) = lua {
        let args = lua.pack_multi(domain)?;
        config::lua::emit_event(&lua, ("gui-attached".to_string(), args)).await?;
    }
    Ok(())
}

async fn trigger_and_log_gui_attached(domain: MuxDomain) {
    if let Err(err) =
        config::with_lua_config_on_main_thread(move |lua| trigger_gui_attached(lua, domain)).await
    {
        let message = format!("while processing gui-attached event: {:#}", err);
        log::error!("{}", message);
        persistent_toast_notification("Error", &message);
    }
}

async fn async_run_terminal_gui(
    cmd: Option<CommandBuilder>,
    opts: StartCommand,
) -> anyhow::Result<()> {
    let unix_socket_path =
        config::RUNTIME_DIR.join(format!("gui-sock-{}", unsafe { libc::getpid() }));
    std::env::set_var("WEZTERM_UNIX_SOCKET", unix_socket_path.clone());

    if !opts.no_auto_connect {
        connect_to_auto_connect_domains().await?;
    }

    let spawn_command = match &cmd {
        Some(cmd) => Some(SpawnCommand::from_command_builder(cmd)?),
        None => None,
    };

    // Apply the domain to the command
    let spawn_command = match (spawn_command, &opts.domain) {
        (Some(spawn), Some(name)) => Some(SpawnCommand {
            domain: SpawnTabDomain::DomainName(name.to_string()),
            ..spawn
        }),
        (None, Some(name)) => Some(SpawnCommand {
            domain: SpawnTabDomain::DomainName(name.to_string()),
            ..SpawnCommand::default()
        }),
        (spawn, None) => spawn,
    };
    let mux = Mux::get();

    let domain = if let Some(name) = &opts.domain {
        let domain = mux
            .get_domain_by_name(name)
            .ok_or_else(|| anyhow!("invalid domain {name}"))?;
        Some(domain)
    } else {
        None
    };

    if !opts.attach {
        trigger_and_log_gui_startup(spawn_command).await;
    }

    let is_connecting = opts.attach;

    if let Some(domain) = &domain {
        if !opts.attach {
            let window_id = {
                // Force the builder to notify the frontend early,
                // so that the attach await below doesn't block it.
                let workspace = None;
                let position = None;
                let builder = mux.new_empty_window(workspace, position);
                *builder
            };

            domain.attach(Some(window_id)).await?;
            let config = config::configuration();
            let dpi = config.dpi.unwrap_or_else(|| ::window::default_dpi()) as u32;
            let tab = domain
                .spawn(config.initial_size(dpi), cmd.clone(), None, window_id)
                .await?;
            let mut window = mux
                .get_window_mut(window_id)
                .ok_or_else(|| anyhow!("failed to get mux window id {window_id}"))?;
            if let Some(tab_idx) = window.idx_by_id(tab.tab_id()) {
                window.set_active_without_saving(tab_idx);
            }
            trigger_and_log_gui_attached(MuxDomain(domain.domain_id())).await;
        }
    }
    spawn_tab_in_domain_if_mux_is_empty(cmd, is_connecting, domain, opts.workspace).await
}

#[derive(Debug)]
enum Publish {
    TryPathOrPublish(PathBuf),
    NoConnectNoPublish,
    NoConnectButPublish,
}

impl Publish {
    pub fn resolve(mux: &Arc<Mux>, config: &ConfigHandle, always_new_process: bool) -> Self {
        if mux.default_domain().domain_name() != config.default_domain.as_deref().unwrap_or("local")
        {
            return Self::NoConnectNoPublish;
        }

        if always_new_process {
            return Self::NoConnectNoPublish;
        }

        if config::is_config_overridden() {
            // They're using a specific config file: assume that it is
            // different from the running gui
            log::trace!("skip existing gui: config is different");
            return Self::NoConnectNoPublish;
        }

        match wezterm_client::discovery::resolve_gui_sock_path(
            &crate::termwindow::get_window_class(),
        ) {
            Ok(path) => Self::TryPathOrPublish(path),
            Err(_) => Self::NoConnectButPublish,
        }
    }

    pub fn try_spawn(
        &mut self,
        cmd: Option<CommandBuilder>,
        config: &ConfigHandle,
        workspace: Option<&str>,
        domain: SpawnTabDomain,
    ) -> anyhow::Result<bool> {
        if let Publish::TryPathOrPublish(gui_sock) = &self {
            let dom = config::UnixDomain {
                socket_path: Some(gui_sock.clone()),
                no_serve_automatically: true,
                ..Default::default()
            };
            let mut ui = mux::connui::ConnectionUI::new_headless();
            match wezterm_client::client::Client::new_unix_domain(None, &dom, false, &mut ui, true)
            {
                Ok(client) => {
                    let executor = promise::spawn::ScopedExecutor::new();
                    let command = cmd.clone();
                    let res = block_on(executor.run(async move {
                        let vers = client.verify_version_compat(&mut ui).await?;

                        if vers.executable_path != std::env::current_exe().context("resolve executable path")? {
                            *self = Publish::NoConnectNoPublish;
                            anyhow::bail!(
                                "Running GUI is a different executable from us, will start a new one");
                        }
                        if vers.config_file_path
                            != std::env::var_os("WEZTERM_CONFIG_FILE").map(Into::into)
                        {
                            *self = Publish::NoConnectNoPublish;
                            anyhow::bail!(
                                "Running GUI has different config from us, will start a new one"
                            );
                        }
                        client
                            .spawn_v2(codec::SpawnV2 {
                                domain,
                                window_id: None,
                                command,
                                command_dir: None,
                                size: config.initial_size(0),
                                workspace: workspace.unwrap_or(
                                    config
                                        .default_workspace
                                        .as_deref()
                                        .unwrap_or(mux::DEFAULT_WORKSPACE)
                                ).to_string(),
                            })
                            .await
                    }));

                    match res {
                        Ok(res) => {
                            log::info!(
                                "Spawned your command via the existing GUI instance. \
                             Use wezterm start --always-new-process if you do not want this behavior. \
                             Result={:?}",
                                res
                            );
                            Ok(true)
                        }
                        Err(err) => {
                            log::trace!(
                                "while attempting to ask existing instance to spawn: {:#}",
                                err
                            );
                            Ok(false)
                        }
                    }
                }
                Err(err) => {
                    // Couldn't connect: it's probably a stale symlink.
                    // That's fine: we can continue with starting a fresh gui below.
                    log::trace!("{:#}", err);
                    Ok(false)
                }
            }
        } else {
            Ok(false)
        }
    }
}

fn setup_mux(
    local_domain: Arc<dyn Domain>,
    config: &ConfigHandle,
    default_domain_name: Option<&str>,
    default_workspace_name: Option<&str>,
) -> anyhow::Result<Arc<Mux>> {
    let mux = Arc::new(mux::Mux::new(Some(local_domain.clone())));
    Mux::set_mux(&mux);
    let client_id = Arc::new(mux::client::ClientId::new());
    mux.register_client(client_id.clone());
    mux.replace_identity(Some(client_id));
    let default_workspace_name = default_workspace_name.unwrap_or(
        config
            .default_workspace
            .as_deref()
            .unwrap_or(mux::DEFAULT_WORKSPACE),
    );
    mux.set_active_workspace(&default_workspace_name);

    let default_name =
        default_domain_name.unwrap_or(config.default_domain.as_deref().unwrap_or("local"));

    let domain = mux.get_domain_by_name(default_name).ok_or_else(|| {
        anyhow::anyhow!(
            "desired default domain '{}' was not found in mux!?",
            default_name
        )
    })?;
    mux.set_default_domain(&domain);

    Ok(mux)
}

fn build_initial_mux(
    config: &ConfigHandle,
    default_domain_name: Option<&str>,
    default_workspace_name: Option<&str>,
) -> anyhow::Result<Arc<Mux>> {
    let domain: Arc<dyn Domain> = Arc::new(LocalDomain::new("local")?);
    setup_mux(domain, config, default_domain_name, default_workspace_name)
}

fn run_terminal_gui(opts: StartCommand, default_domain_name: Option<String>) -> anyhow::Result<()> {
    if let Some(cls) = opts.class.as_ref() {
        crate::set_window_class(cls);
    }
    if let Some(pos) = opts.position.as_ref() {
        set_window_position(pos.clone());
    }

    let config = config::configuration();
    let need_builder = !opts.prog.is_empty() || opts.cwd.is_some();

    let cmd = if need_builder {
        let prog = opts.prog.iter().map(|s| s.as_os_str()).collect::<Vec<_>>();
        let mut builder = config.build_prog(
            if prog.is_empty() { None } else { Some(prog) },
            config.default_prog.as_ref(),
            config.default_cwd.as_ref(),
        )?;
        if let Some(cwd) = &opts.cwd {
            builder.cwd(if cwd.is_relative() {
                current_dir()?.join(cwd).into_os_string().into()
            } else {
                Cow::Borrowed(cwd.as_ref())
            });
        }
        Some(builder)
    } else {
        None
    };

    let mux = build_initial_mux(
        &config,
        default_domain_name.as_deref(),
        opts.workspace.as_deref(),
    )?;

    // First, let's see if we can ask an already running wezterm to do this.
    // We must do this before we start the gui frontend as the scheduler
    // requirements are different.
    let mut publish = Publish::resolve(
        &mux,
        &config,
        opts.always_new_process || opts.position.is_some(),
    );
    log::trace!("{:?}", publish);
    if publish.try_spawn(
        cmd.clone(),
        &config,
        opts.workspace.as_deref(),
        match &opts.domain {
            Some(name) => SpawnTabDomain::DomainName(name.to_string()),
            None => SpawnTabDomain::DefaultDomain,
        },
    )? {
        return Ok(());
    }

    let gui = crate::frontend::try_new()?;
    let activity = Activity::new();

    promise::spawn::spawn(async move {
        if let Err(err) = async_run_terminal_gui(cmd, opts).await {
            terminate_with_error(err);
        }
        drop(activity);
    })
    .detach();

    maybe_show_configuration_error_window();
    gui.run_forever()
}

fn fatal_toast_notification(title: &str, message: &str) {
    persistent_toast_notification(title, message);
    // We need a short delay otherwise the notification
    // will not show
    #[cfg(windows)]
    std::thread::sleep(std::time::Duration::new(2, 0));
}

fn notify_on_panic() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(s) = info.payload().downcast_ref::<&str>() {
            fatal_toast_notification("Wezterm panic", s);
        }
        default_hook(info);
    }));
}

fn terminate_with_error_message(err: &str) -> ! {
    log::error!("{}; terminating", err);
    fatal_toast_notification("Wezterm Error", &err);
    std::process::exit(1);
}

fn terminate_with_error(err: anyhow::Error) -> ! {
    let mut err_text = format!("{err:#}");

    let warnings = config::configuration_warnings_and_errors();
    if !warnings.is_empty() {
        let err = warnings.join("\n");
        err_text = format!("{err_text}\nConfiguration Error: {err}");
    }

    terminate_with_error_message(&err_text)
}

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    config::designate_this_as_the_main_thread();
    config::assign_error_callback(mux::connui::show_configuration_error_message);
    notify_on_panic();
    if let Err(e) = run() {
        terminate_with_error(e);
    }
    Mux::shutdown();
    frontend::shutdown();
}

fn maybe_show_configuration_error_window() {
    let warnings = config::configuration_warnings_and_errors();
    if !warnings.is_empty() {
        let err = warnings.join("\n");
        mux::connui::show_configuration_error_message(&err);
    }
}

fn run() -> anyhow::Result<()> {
    // Inform the system of our AppUserModelID.
    // Without this, our toast notifications won't be correctly
    // attributed to our application.
    #[cfg(windows)]
    {
        unsafe {
            ::windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
                ::windows::core::PCWSTR(wide_string("org.wezfurlong.wezterm").as_ptr()),
            )
            .unwrap();
        }
    }

    let opts = Opt::parse();

    // This is a bit gross.
    // In order to not to automatically open a standard windows console when
    // we run, we use the windows_subsystem attribute at the top of this
    // source file.  That comes at the cost of causing the help output
    // to disappear if we are actually invoked from a console.
    // This AttachConsole call will attach us to the console of the parent
    // in that situation, but since we were launched as a windows subsystem
    // application we will be running asynchronously from the shell in
    // the command window, which means that it will appear to the user
    // that we hung at the end, when in reality the shell is waiting for
    // input but didn't know to re-draw the prompt.
    #[cfg(windows)]
    unsafe {
        if opts.attach_parent_console {
            winapi::um::wincon::AttachConsole(winapi::um::wincon::ATTACH_PARENT_PROCESS);
        }
    };

    env_bootstrap::bootstrap();

    let _saver = umask::UmaskSaver::new();

    config::common_init(
        opts.config_file.as_ref(),
        &opts.config_override,
        opts.skip_config,
    )?;
    let config = config::configuration();

    let sub = match opts.cmd.as_ref().cloned() {
        Some(sub) => sub,
        None => {
            // Need to fake an argv0
            let mut argv = vec!["wezterm-gui".to_string()];
            for a in &config.default_gui_startup_args {
                argv.push(a.clone());
            }
            SubCommand::try_parse_from(&argv).with_context(|| {
                format!(
                    "parsing the default_gui_startup_args config: {:?}",
                    config.default_gui_startup_args
                )
            })?
        }
    };

    match sub {
        SubCommand::Start(start) => {
            log::trace!("Using configuration: {:#?}\nopts: {:#?}", config, opts);
            run_terminal_gui(start, None)
        }
    }
}
