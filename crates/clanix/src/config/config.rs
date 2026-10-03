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
// use serde_json;
use toml;

use crate::ConfigError;

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
    pub(crate) timeout_ms: Duration,
    pub(crate) working_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBuilder {
    disable_timeout: bool,
    timeout_ms: u128,
    working_dir: PathBuf,
}

impl Config {
     /* Incase of Config, new() gives the new instance of what already exists on disk,
     * you can call defaults() on ConfigBuilder if you explicitly want the instance with
     * defaults
     * 
     * Also there are 2 identical methods where both do `ConfigBuilder::read()`
     * These are here just for the sake of clarity and I might remove it in future 
     */
     
    pub fn new() -> Result<ConfigBuilder, ConfigError> {
        ConfigBuilder::read()
    }

    pub fn read() -> Result<ConfigBuilder, ConfigError> {
        ConfigBuilder::read()
    }

    pub fn set_default() -> Result<ConfigBuilder, ConfigError> {
        ConfigBuilder::defaults()?.generate()
    }
    
    /// Default timeout_ms of 1ms is just for experimentation
    /// and will be changed in future so it's more realistic
    pub fn default_timeout_ms() -> Duration {
        Duration::from_millis(1)
    }
    
    pub fn default_working_dir() -> Result<PathBuf, ConfigError> {
        env::current_dir()
            .map_err(|_| ConfigError::WorkingDir)
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

    pub fn new() -> Result<Self, ConfigError> {
        Self::read()
    }
    
    pub fn defaults() -> Result<Self, ConfigError> {
        Ok(Self {
            disable_timeout: Config::default_disable_timeout(),
            timeout_ms: Config::default_timeout_ms().as_millis(),
            working_dir: Config::default_working_dir()?
        })
    }
    
    pub fn set_default() -> Result<Self, ConfigError> {
        Self::defaults()?.generate()
    }

    pub fn set_disable_timeout(mut self, disable: Option<bool>) -> Result<Self, ConfigError> {
        match disable {
            Some(dis) => {
                self.disable_timeout = dis;
                self.generate()
            },
            None => Ok(self)
        }
    }

    pub fn timeout_ms(&self) -> Duration {
        Duration::from_millis(self.timeout_ms as u64)
    }
    
    pub fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }
    
    pub fn working_dir(&self) -> &PathBuf {
        &self.working_dir
    }
    
    pub fn set_timeout_ms(mut self, timeout_ms: Option<u64>) -> Result<Self, ConfigError> {
        match timeout_ms {
            Some(timeout) => {
                self.timeout_ms = timeout as u128;
                self.generate() // make changes to disk
            },
            None => return Ok(self)
        }
    }

    pub fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Result<Self, ConfigError> {
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
    /// Here, we dont write the instance of Config because the timeout_ms
    /// inside Config which we use to do operations is in Duration form,
    /// because it allows us to do various kinds of stuff
    /// but we dont want to store Duration struct instance on disk,
    /// we want a clean timeout_ms: u128.
    pub fn generate(self) -> Result<Self, ConfigError> {
        let config = toml::to_string_pretty(
            &ConfigBuilder {
                disable_timeout: self.disable_timeout,
                timeout_ms: self.timeout_ms,
                working_dir: self.working_dir.clone()
            }
        )
        .map_err(|_| ConfigError::Prepare)?;

        fs::write(&NAME_TOML, config)
            .map_err(|_| ConfigError::Write)?;
        
        Ok(self)
    }

    /// Read the config from disk which is stored in the form of builder struct
    /// and if the file doesn't exist, return the default config.
    /// 
    /// This doesn't create or change the config on disk
    pub fn read() -> Result<Self, ConfigError> {
        let config = if PathBuf::from(NAME_TOML).exists() {
            // let file = fs::File::open(NAME_TOML)
            //     .map_err(ConfigError::Open)?;
            
            let content = fs::read_to_string(NAME_TOML)
                .map_err(ConfigError::Read)?;
            toml::from_str(&content)
                .map_err(|_| ConfigError::Parse)? 
            
        } else {
            Self::defaults()?.generate()?
        };
        Ok(config)
    }
}
