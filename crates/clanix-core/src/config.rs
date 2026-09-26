
use std::env;
use std::io::BufReader;
use std::path::{PathBuf, Path};
use std::fs;
use serde::{Serialize, Deserialize};
use serde_json;

use super::Policy;

const NAME: &str = "clanix.json";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Config {
    timeout: bool,
    timeout_ms: u128,
    default_working_dir: PathBuf,
}

impl Config {
    pub(super) fn timeout_ms(&self) -> u128 {
        self.timeout_ms
    }

    pub(super) fn default_working_dir(&self) -> &Path {
        &self.default_working_dir
    }

    pub(super) fn default_timeout_enable(&self) -> bool {
        self.timeout
    }

    fn defaults() -> Self {
        Self {
            timeout: true,
            timeout_ms: 100,
            default_working_dir: env::current_dir().unwrap(), // replace it later
        }
    }
    
    pub(super) fn new() -> Self {
        Self::defaults()
    }
    
    pub(super) fn generate(&self, policy: &Policy) {
        let _config = Self {
            timeout: policy.timeout_enable,
            timeout_ms: policy.timeout_ms.as_millis(),
            default_working_dir: policy.working_dir.clone()
        };
        let config = serde_json::to_string_pretty(&_config).unwrap();

        fs::write(&NAME, &config).unwrap();
    }

    pub(super) fn read() -> Self {
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
