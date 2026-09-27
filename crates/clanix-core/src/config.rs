
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

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    disable_timeout: bool,
    timeout_ms: Duration,
    working_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TempConfig {
    disable_timeout: bool,
    timeout_ms: u128,
    working_dir: PathBuf,
}


/*
 * Call of any method of Config changes persistent state, aka on disk
 */
impl PolicyTrait for Config {
    fn defaults() -> Self {
        Self {
            disable_timeout: Self::default_disable_timeout(),
            timeout_ms: Self::default_timeout_ms(),
            working_dir: Self::default_working_dir() 
        }
    }

    fn timeout_ms(&self) -> Duration {
        self.timeout_ms
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
                self.timeout_ms = Duration::from_millis(timeout);
                return Self::generate(self); // make changes to disk
            },
            None => return self
        }
    }

    fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Self {
        match working_dir {
            Some(dir) => {
                self.working_dir = dir;
                return Self::generate(self); // make changes to disk
            },
            None => return self
        }
    }

    fn set_disable_timeout(mut self, disable: bool) -> Self {
        if !disable {return self;}

        self.disable_timeout = disable;
        self.generate()
    }
}

impl Config {
    pub fn set_default(default: bool) -> Self {
        /*
         * If user passed default: true,
         * set the default in config(disk) and return
         * Self, if not, just return Self without
         * making change in disk/config
         */
        
        if default {
            Config::defaults()
                .generate()
        } else {
            Config::defaults()
        }
    }
    
    pub fn generate(self) -> Self {
        let config = serde_json::to_string_pretty(
            &TempConfig {
                disable_timeout: self.disable_timeout,
                timeout_ms: self.timeout_ms.as_millis(),
                working_dir: self.working_dir.clone()
            }
        ).unwrap();

        fs::write(&NAME, config).unwrap();
        self
    }

    pub fn read() -> Self {
        if !PathBuf::from(NAME).exists() {
            println!("{NAME} doesn't exist..\nDefaulting to fallback values..");
            return Self::defaults()
        }
        
        let file = fs::File::open(NAME).unwrap();
        let reader = BufReader::new(file);
        let config = serde_json::from_reader(reader).unwrap();
        config
    }
}
