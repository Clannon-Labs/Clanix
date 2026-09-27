
use std::io::BufReader;
use std::path::PathBuf;
use std::fs;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use serde_json;

use super::{
    shared::PolicyTrait,
};

const NAME: &str = "clanix.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    disable_timeout: bool,
    timeout_ms: Duration,
    working_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBuilder {
    disable_timeout: bool,
    timeout_ms: u128,
    working_dir: PathBuf,
}


impl Config {
    /*
     * Incase of Config, new() gives the new instance of what already exists on disk,
     * you can call defaults() on ConfigBuilder if you explicitly want defaults
     * 
     * The reason behind keeping 2 functions however; is clarity
     */
    pub fn new() -> ConfigBuilder {
        ConfigBuilder::read()
    }

    pub fn set_default(default: bool) -> ConfigBuilder {
        ConfigBuilder::set_default(default)
    }
    
}

/*
 * Call of any method of Config changes persistent state, aka on disk
 */
impl PolicyTrait for ConfigBuilder {
    fn defaults() -> Self {
        Self {
            disable_timeout: ConfigBuilder::default_disable_timeout(),
            timeout_ms: ConfigBuilder::default_timeout_ms().as_millis(),
            working_dir: ConfigBuilder::default_working_dir() 
        }
    }

    fn timeout_ms(&self) -> Duration {
        Duration::from_millis(self.timeout_ms as u64)
    }
    
    fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }
    
    fn working_dir(&self) -> &PathBuf {
        &self.working_dir
    }
    
    fn set_timeout_ms(mut self, timeout_ms: Option<u64>) -> Self {
        match timeout_ms {
            Some(timeout) => {
                self.timeout_ms = timeout as u128;
                self.generate() // make changes to disk
            },
            None => return self
        }
    }

    fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Self {
        match working_dir {
            Some(dir) => {
                self.working_dir = dir;
                self.generate() // make changes to disk
            },
            None => return self
        }
    }
}

impl ConfigBuilder {
    pub fn set_default(default: bool) -> Self {
        /*
         * If user passed default: true,
         * set the default in config(disk) and return
         * Self, if not, just return Self without
         * making change in disk/config
         */
        
        if default {
            Self::defaults()
                .generate()
        } else {
            Self::defaults()
        }
    }

    pub fn set_disable_timeout(mut self, disable: Option<bool>) -> Self {
        match disable {
            Some(dis) => {
                self.disable_timeout = dis;
                self.generate()
            },
            None => self
        }
    }
    
    pub fn generate(self) -> Self {
        let config = serde_json::to_string_pretty(
            &ConfigBuilder {
                disable_timeout: self.disable_timeout,
                timeout_ms: self.timeout_ms,
                working_dir: self.working_dir.clone()
            }
        ).unwrap();

        fs::write(&NAME, config).unwrap();
        self
    }

    pub fn read() -> ConfigBuilder {
        if !PathBuf::from(NAME).exists() {
            println!("{NAME} doesn't exist..\nDefaulting to fallback values..");
            return ConfigBuilder::defaults()
        }
        
        let file = fs::File::open(NAME).unwrap();
        let reader = BufReader::new(file);
        let config = serde_json::from_reader(reader).unwrap();
        config
    }
}
