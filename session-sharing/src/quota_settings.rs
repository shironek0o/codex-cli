use std::fs::{self, DirBuilder};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use crate::{PrivateDirectory, Result};

#[derive(Clone, Copy)]
pub enum Setting {
    Switch,
    Resume,
}

pub fn codex_home() -> Result<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| "cannot locate CODEX_HOME".into())
}

pub fn home_for_socket(socket: &Path) -> Result<&Path> {
    socket
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "invalid daemon socket path".into())
}

impl Setting {
    fn name(self) -> &'static str {
        match self {
            Self::Switch => "auto-switch",
            Self::Resume => "auto-resume",
        }
    }

    fn override_var(self) -> &'static str {
        match self {
            Self::Switch => "CODEX_AUTO_SWITCH_OFF",
            Self::Resume => "CODEX_AUTO_RESUME_OFF",
        }
    }

    fn path(self, home: &Path) -> PathBuf {
        home.join(format!("session-sharing-{}", self.name()))
    }

    fn saved_enabled(self, home: &Path) -> bool {
        match fs::read(self.path(home)) {
            Ok(value) => value == b"on\n",
            // Upgrade both settings from the previous combined opt-out.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                !home.join("session-sharing-auto-switch-off").is_file()
            }
            Err(_) => false,
        }
    }

    pub fn enabled(self, home: &Path) -> bool {
        std::env::var_os(self.override_var()).is_none() && self.saved_enabled(home)
    }

    pub fn command(self, args: &[String]) -> Result<i32> {
        let home = codex_home()?;
        match args {
            [] => {}
            [arg] if arg == "status" => {}
            [arg] if matches!(arg.as_str(), "on" | "off") => {
                DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&home)?;
                let staging = PrivateDirectory::create_in(&home)?;
                let path = staging.0.join("setting");
                fs::write(&path, format!("{arg}\n"))?;
                fs::rename(path, self.path(&home))?;
            }
            _ => {
                eprintln!("Usage: codex {} [status|on|off]", self.name());
                return Ok(2);
            }
        }
        let label = match self {
            Self::Switch => "Automatic account switching",
            Self::Resume => "Automatic continuation after quota reset",
        };
        let state = if self.enabled(&home) { "on" } else { "off" };
        let variable = self.override_var();
        if std::env::var_os(variable).is_some() {
            println!("{label}: {state} ({variable} is set)");
        } else {
            println!("{label}: {state}");
        }
        Ok(0)
    }
}
