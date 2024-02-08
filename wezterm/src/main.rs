use anyhow::anyhow;
use clap::builder::ValueParser;
use clap::{Parser, ValueEnum, ValueHint};
use clap_complete::{generate as generate_completion, shells, Generator as CompletionGenerator};
use config::wezterm_version;
use mux::Mux;
use std::ffi::OsString;
use umask::UmaskSaver;
use wezterm_gui_subcommands::*;

//    let message = "; ❤ 😍🤢\n\x1b[91;mw00t\n\x1b[37;104;m bleet\x1b[0;m.";

#[derive(Debug, Parser)]
#[command(
    about = "Wez's Terminal Emulator\nhttp://github.com/wez/wezterm",
    version = wezterm_version()
)]
pub struct Opt {
    /// Skip loading wezterm.lua
    #[arg(long, short = 'n')]
    skip_config: bool,

    /// Specify the configuration file to use, overrides the normal
    /// configuration file resolution
    #[arg(
        long,
        value_parser,
        conflicts_with = "skip_config",
        value_hint=ValueHint::FilePath
    )]
    config_file: Option<OsString>,

    /// Override specific configuration values
    #[arg(
        long = "config",
        name = "name=value",
        value_parser=ValueParser::new(name_equals_value),
        number_of_values = 1)]
    config_override: Vec<(String, String)>,

    #[command(subcommand)]
    cmd: Option<SubCommand>,
}

#[derive(Debug, Clone, ValueEnum)]
enum Shell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
    Fig,
}

impl CompletionGenerator for Shell {
    fn file_name(&self, name: &str) -> String {
        match self {
            Shell::Bash => shells::Bash.file_name(name),
            Shell::Elvish => shells::Elvish.file_name(name),
            Shell::Fish => shells::Fish.file_name(name),
            Shell::PowerShell => shells::PowerShell.file_name(name),
            Shell::Zsh => shells::Zsh.file_name(name),
            Shell::Fig => clap_complete_fig::Fig.file_name(name),
        }
    }

    fn generate(&self, cmd: &clap::Command, buf: &mut dyn std::io::Write) {
        match self {
            Shell::Bash => shells::Bash.generate(cmd, buf),
            Shell::Elvish => shells::Elvish.generate(cmd, buf),
            Shell::Fish => shells::Fish.generate(cmd, buf),
            Shell::PowerShell => shells::PowerShell.generate(cmd, buf),
            Shell::Zsh => shells::Zsh.generate(cmd, buf),
            Shell::Fig => clap_complete_fig::Fig.generate(cmd, buf),
        }
    }
}

#[derive(Debug, Parser, Clone)]
enum SubCommand {
    #[command(
        name = "start",
        about = "Start the GUI, optionally running an alternative program"
    )]
    Start(StartCommand),

    /// Generate shell completion information
    #[command(name = "shell-completion")]
    ShellCompletion {
        /// Which shell to generate for
        #[arg(long, value_parser)]
        shell: Shell,
    },
}

fn terminate_with_error_message(err: &str) -> ! {
    log::error!("{}; terminating", err);
    std::process::exit(1);
}

fn terminate_with_error(err: anyhow::Error) -> ! {
    terminate_with_error_message(&format!("{:#}", err));
}

fn main() {
    config::designate_this_as_the_main_thread();
    config::assign_error_callback(mux::connui::show_configuration_error_message);
    if let Err(e) = run() {
        terminate_with_error(e);
    }
    Mux::shutdown();
}

fn run() -> anyhow::Result<()> {
    env_bootstrap::bootstrap();

    let saver = UmaskSaver::new();

    let opts = Opt::parse();

    match opts
        .cmd
        .as_ref()
        .cloned()
        .unwrap_or_else(|| SubCommand::Start(StartCommand::default()))
    {
        SubCommand::Start(_) => delegate_to_gui(saver),
        SubCommand::ShellCompletion { shell } => {
            use clap::CommandFactory;
            let mut cmd = Opt::command();
            let name = cmd.get_name().to_string();
            generate_completion(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
    }
}

fn delegate_to_gui(saver: UmaskSaver) -> anyhow::Result<()> {
    use std::process::Command;

    // Restore the original umask
    drop(saver);

    let exe_name = if cfg!(windows) {
        "wezterm-gui.exe"
    } else {
        "wezterm-gui"
    };

    let exe = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("exe has no parent dir!?"))?
        .join(exe_name);

    let mut cmd = Command::new(exe);
    if cfg!(windows) {
        cmd.arg("--attach-parent-console");
    }

    cmd.args(std::env::args_os().skip(1));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Clean up random fds, except when we're running in an AppImage.
        // AppImage relies on child processes keeping alive an fd that
        // references the mount point and if we close it as part of execing
        // the gui binary, the appimage gets unmounted before we can exec.
        if std::env::var_os("APPIMAGE").is_none() {
            portable_pty::unix::close_random_fds();
        }
        let res = cmd.exec();
        return Err(anyhow::anyhow!("failed to exec {cmd:?}: {res:?}"));
    }

    #[cfg(windows)]
    {
        let mut child = cmd.spawn()?;
        let status = child.wait()?;
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }
}
