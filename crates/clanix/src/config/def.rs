//! Config is the actual persistent policy version you use to set the persistent
//! values aka the values that get written to disk
//!
//! It shares the same fields as Policy

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use humantime_serde::{self, re::humantime};
use serde::{Deserialize, Serialize};

use crate::ConfigError;

type Result<T> = std::result::Result<T, ConfigError>;

#[allow(unused)]
const NAME_JSON: &str = "clanix.json";

#[allow(unused)]
const NAME_TOML: &str = "clanix.toml";

/// The main config that provides the Config to the clients
/// which they can use to operate on the config without having to worry
/// about the internal messy structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub(crate) disable_timeout: bool,

    #[serde(with = "humantime_serde")]
    pub(crate) timeout: Duration,

    pub(crate) working_dir: Option<PathBuf>,
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nConfig")?;
        writeln!(f, "-------")?;

        writeln!(f, "Timeout:-")?;
        if self.disable_timeout() {
            writeln!(f, " > Disabled")?;
        } else {
            writeln!(f, " > Enabled")?;
        }

        write!(f, " > Threshold:")?;
        writeln!(f, "{}", humantime::format_duration(self.timeout))?;

        writeln!(f, "\nWorking directory: {:#?}\n", self.working_dir)?;

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Config {
        Config {
            disable_timeout: Config::default_disable_timeout(),
            timeout: Config::default_timeout(),
            working_dir: Config::default_working_dir(),
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_default() -> Result<Self> {
        let _ = Self::default().save()?;
        Ok(Self::default())
    }

    pub fn default_timeout() -> Duration {
        Duration::from_secs(60)
    }

    // It starts with no working directory and defines it in the runtime
    // in other methods where we already return errors
    pub fn default_working_dir() -> Option<PathBuf> {
        None
    }

    /// timeout is enabled (or not disabled) by default
    pub fn default_disable_timeout() -> bool {
        false
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }

    pub fn working_dir(&self) -> Option<&PathBuf> {
        self.working_dir.as_ref()
    }

    /// Setters with a fallback to old value if save failed
    pub fn set_disable_timeout(&mut self, disable: bool) -> Result<&mut Self> {
        if self.disable_timeout() != disable {
            self.disable_timeout = disable;
            let _ = self.save()?;
            Ok(self)
        } else {
            Ok(self)
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) -> Result<&mut Self> {
        let old = self.timeout();
        if old != timeout {
            self.timeout = timeout;
            match self.save() {
                Ok(_) => Ok(self),
                Err(_) => {
                    self.timeout = old;
                    Ok(self)
                }
            }
        } else {
            Ok(self)
        }
    }

    pub fn set_working_dir(&mut self, working_dir: PathBuf) -> Result<&mut Self> {
        let old = self.working_dir.clone();
        match self.working_dir() {
            Some(dir) if *dir != working_dir => {
                self.working_dir = Some(working_dir);
                match self.save() {
                    Ok(_) => Ok(self),
                    Err(_) => {
                        self.working_dir = old;
                        Ok(self)
                    }
                }
            }
            _ => Ok(self),
        }
    }

    pub fn clear_working_dir(&mut self) -> Result<&mut Self> {
        self.working_dir = None;
        let _ = self.save()?;
        Ok(self)
    }
}

impl Config {
    pub fn save(&self) -> Result<&Self> {
        let config = toml::to_string_pretty(self).map_err(ConfigError::PrepareToml)?;

        fs::write(NAME_TOML, config).map_err(|source| ConfigError::Write {
            path: PathBuf::from(NAME_TOML),
            source,
        })?;

        Ok(self)
    }

    /// Read the config from disk which is stored in the form of builder struct
    /// and if the file doesn't exist, return the default config.
    ///
    /// This doesn't create or change the config on disk
    pub fn read() -> Result<Self> {
        let config = if PathBuf::from(NAME_TOML).exists() {
            let content = fs::read_to_string(NAME_TOML).map_err(|source| ConfigError::Read {
                path: PathBuf::from(NAME_TOML),
                source,
            })?;
            toml::from_str(&content).map_err(ConfigError::ParseToml)?
        } else {
            Self::set_default()?
        };
        Ok(config)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(ConfigError::JsonError)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(ConfigError::PrepareToml)
    }
}
