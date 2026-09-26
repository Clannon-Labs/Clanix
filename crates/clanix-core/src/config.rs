
use std::io::BufReader;
use std::path::{PathBuf, Path};
use std::fs;
use serde::{Serialize, Deserialize};
use serde_json;

use super::{
    shared::PolicyTrait,
};

const NAME: &str = "clanix.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    timeout_disable: bool,
    timeout_ms: u128,
    working_dir: PathBuf,
}

impl PolicyTrait for Config {
    fn defaults() -> Self {
        Self {
            timeout_disable: Self::default_timeout_disable(),
            timeout_ms: Self::default_timeout_ms().as_millis(),
            working_dir: Self::default_working_dir() 
        }
    }

    fn set_timeout_ms(mut self, timeout_ms: Option<u64>) -> Self {
        match timeout_ms {
            Some(timeout) => {
                self.timeout_ms = timeout as u128;
                Self::generate(&self); // make changes to disk
                return self
            },
            None => return self
        }
    }

    fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Self {
        match working_dir {
            Some(dir) => {
                self.working_dir = dir;
                Self::generate(&self); // make changes to disk
                return self
            },
            None => return self
        }
    }
}

impl Config {
    pub fn timeout_ms(&self) -> u128 {
        self.timeout_ms
    }

    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    pub fn timeout_disable(&self) -> bool {
        self.timeout_disable
    }

    pub fn set_default(default: bool) -> Self {
        /*
         * If user passed default: true,
         * set the default in config and return
         * Self, if not, just return Self without
         * making change in disk/config
         */
        let default_config = Self::defaults();
        
        if default {
            let _ = default_config
                .generate();
        }

        default_config
    }

    pub fn set_timeout_disable(mut self, timeout_disable: Option<bool>) -> Self {
        match timeout_disable {
            Some(disable) => {
                self.timeout_disable = disable;
                Self::generate(&self); // make changes to disk
                return self
            },
            None => return self
        }
    }
    
    pub fn generate(&self) -> &Self {
        let config = serde_json::to_string_pretty(
            &Self {
                timeout_disable: self.timeout_disable,
                timeout_ms: self.timeout_ms,
                working_dir: self.working_dir.clone()
            }
        ).unwrap();

        fs::write(&NAME, &config).unwrap();
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
