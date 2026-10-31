//! Config also implements the builder pattern and
//! provides the benefits that the ExecutionRequest also provides.
//! 
//! Config is the actual persistent policy version you use to set the persistent
//! values
//! 
//! It shares the same PolicyTrait trait as policy

use std::fs;
use std::env;
// use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Serialize, Deserialize};
use serde_json;
use toml;

use crate::ConfigError;

type Result<T> = std::result::Result<T, ConfigError>;

#[allow(unused)]
const NAME_JSON: &str = "clanix.json";

#[allow(unused)]
const NAME_TOML: &str = "clanix.toml";

/// The main config that provides the ConfigBuilder to the clients
/// which they can use to operate on the config without having to worry
/// about the internal messy structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub(crate) disable_timeout: bool,
    pub(crate) timeout: Duration,
    pub(crate) working_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBuilder {
    disable_timeout: bool,
    timeout: Duration,
    working_dir: PathBuf,
}

impl std::fmt::Display for ConfigBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nConfig")?;
        writeln!(f, "-------")?;

        writeln!(f, "Timeout:-")?;
        if self.disable_timeout() {
            writeln!(f, " > Disabled")?;
        } else {
            writeln!(f, " > Enabled")?;
        }
        
        // writeln!(f, "Disable timeout? {}", self.disable_timeout)?;
        
        write!(f, " > Threshold:")?;
        writeln!(f, " {}s {}ms", self.timeout.as_secs(), self.timeout.subsec_millis())?;

        writeln!(f, "\nWorking directory: {}\n", self.working_dir.display())?;
        
        Ok(())
    }
}

impl Config {
     /* Incase of Config, new() gives the new instance of what already exists on disk,
     * you can call default() on ConfigBuilder if you explicitly want the instance with
     * defaults
     * 
     * Also there are 2 identical methods where both do `ConfigBuilder::read()`
     * These are here just for the sake of clarity and I might remove it in future 
     */
     
    pub fn new() -> Result<ConfigBuilder> {
        ConfigBuilder::read()
    }

    pub fn read() -> Result<ConfigBuilder> {
        ConfigBuilder::read()
    }

    pub fn set_default() -> Result<ConfigBuilder> {
        ConfigBuilder::default()?.generate()
    }
    
    /// Default timeout of 1ms is just for experimentation
    /// and will be changed in future so it's more realistic
    pub fn default_timeout() -> Duration {
        Duration::from_secs(60)
    }
    
    pub fn default_working_dir() -> Result<PathBuf> {
        env::current_dir()
            .map_err(|e| ConfigError::WorkingDir(e))
    }

    /// timeout is enabled (or not disabled) by default
    pub fn default_disable_timeout() -> bool {
        false 
    }    
}


impl ConfigBuilder {
    /// If user passed default: true, set_default()
    /// sets the default in config(disk) and return
    /// ConfigBuilder, if not, it just returns ConfigBuilder
    /// without making change in disk/config

    pub fn new() -> Result<Self> {
        Self::read()
    }
    
    pub fn default() -> Result<Self> {
        Ok(Self {
            disable_timeout: Config::default_disable_timeout(),
            timeout: Config::default_timeout(),
            working_dir: Config::default_working_dir()?
        })
    }
    
    pub fn set_default() -> Result<Self> {
        Self::default()?.generate()
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| ConfigError::JsonError(e))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|e| ConfigError::PrepareToml(e))
    }
    
    pub fn set_disable_timeout(mut self, disable: Option<bool>) -> Result<Self> {
        match disable {
            Some(dis) => {
                self.disable_timeout = dis;
                self.generate()
            },
            None => Ok(self)
        }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    
    pub fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }
    
    pub fn working_dir(&self) -> &PathBuf {
        &self.working_dir
    }
    
    pub fn set_timeout(mut self, timeout: Option<Duration>) -> Result<Self> {
        match timeout {
            Some(time) => {
                self.timeout = time;
                self.generate() // make changes to disk
            },
            None => return Ok(self)
        }
    }

    pub fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Result<Self> {
        match working_dir {
            Some(dir) => {
                self.working_dir = dir;
                self.generate() // make changes to disk
            },
            None => return Ok(self)
        }
    }
    
    /// Create the ConfigBuilder struct instance and write on disk
    /// 
    /// Here, we dont write the instance of Config because the timeout
    /// inside Config which we use to do operations is in Duration form,
    /// because it allows us to do various kinds of stuff
    /// but we dont want to store Duration struct instance on disk,
    /// we want a clean timeout: u128.
    pub fn generate(self) -> Result<Self> {
        let config = toml::to_string_pretty(
            &ConfigBuilder {
                disable_timeout: self.disable_timeout,
                timeout: self.timeout,
                working_dir: self.working_dir.clone()
            }
        )
        .map_err(ConfigError::PrepareToml)?;

        fs::write(&NAME_TOML, config)
            .map_err(|source| ConfigError::Write {
                path: PathBuf::from(NAME_TOML),
                source
            })?;
        
        Ok(self)
    }

    /// Read the config from disk which is stored in the form of builder struct
    /// and if the file doesn't exist, return the default config.
    /// 
    /// This doesn't create or change the config on disk
    pub fn read() -> Result<Self> {
        let config = if PathBuf::from(NAME_TOML).exists() {
            // let file = fs::File::open(NAME_TOML)
            //     .map_err(ConfigError::Open)?;
            
            let content = fs::read_to_string(NAME_TOML)
                .map_err(|source| ConfigError::Read {
                    path: PathBuf::from(NAME_TOML),
                    source
                })?;
            toml::from_str(&content)
                .map_err(ConfigError::ParseToml)? 
            
        } else {
            Self::default()?.generate()?
        };
        Ok(config)
    }
}
