use crate::config::validate_domain_name;
use crate::*;
use luahelper::impl_lua_conversion_dynamic;
use std::fmt::Display;
use std::str::FromStr;
use wezterm_dynamic::{FromDynamic, ToDynamic};

#[derive(Debug, Clone, Copy, FromDynamic, ToDynamic)]
pub enum SshBackend {
    Ssh2,
    LibSsh,
}

impl Default for SshBackend {
    fn default() -> Self {
        Self::LibSsh
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromDynamic, ToDynamic)]
pub enum SshMultiplexing {
    WezTerm,
    None,
    // TODO: Tmux-cc in the future?
}

impl Default for SshMultiplexing {
    fn default() -> Self {
        Self::WezTerm
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromDynamic, ToDynamic)]
pub enum Shell {
    /// Unknown command shell: no assumptions can be made
    Unknown,

    /// Posix shell compliant, such that `cd DIR ; exec CMD` behaves
    /// as it does in the bourne shell family of shells
    Posix,
    // TODO: Cmd, PowerShell in the future?
}

impl Default for Shell {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug)]
pub struct SshParameters {
    pub username: Option<String>,
    pub host_and_port: String,
}

impl Display for SshParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(user) = &self.username {
            write!(f, "{}@{}", user, self.host_and_port)
        } else {
            write!(f, "{}", self.host_and_port)
        }
    }
}

pub fn username_from_env() -> anyhow::Result<String> {
    #[cfg(unix)]
    const USER: &str = "USER";
    #[cfg(windows)]
    const USER: &str = "USERNAME";

    std::env::var(USER).with_context(|| format!("while resolving {} env var", USER))
}

impl FromStr for SshParameters {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('@').collect();

        if parts.len() == 2 {
            Ok(Self {
                username: Some(parts[0].to_string()),
                host_and_port: parts[1].to_string(),
            })
        } else if parts.len() == 1 {
            Ok(Self {
                username: None,
                host_and_port: parts[0].to_string(),
            })
        } else {
            bail!("failed to parse ssh parameters from `{}`", s);
        }
    }
}
