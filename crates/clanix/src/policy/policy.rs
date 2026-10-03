
use std::time::Duration;
use std::path::{PathBuf, Path};

use serde::{Serialize, Deserialize};

use crate::config::Config;
use crate::PolicyError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub(crate) disable_timeout: bool,
    pub(crate) timeout_ms: Duration,
    pub(crate) working_dir: PathBuf,
}

/*
 * Calling any method on policy changes data only in memory
 * so if you want to make the change persistent, aka change 
 * in disk, you MUST call the Config methods, not Policy ones
 */
impl Policy {
    pub fn defaults() -> Result<Self, PolicyError> {
        Ok(Self {
            disable_timeout: Config::default_disable_timeout(),
            timeout_ms: Config::default_timeout_ms(),
            working_dir: Config::default_working_dir()?,
        })
    }

    // Just an alias for defaults()
    pub fn new() -> Result<Self, PolicyError> {
        Self::defaults()
    }


    pub fn timeout_ms(&self) -> Duration {
        self.timeout_ms
    }
    
    pub fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }
    
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }
    
    pub fn set_timeout_ms(mut self, timeout_ms: Option<u64>) -> Result<Self, PolicyError> {
        /*
         * If user gives the timeout_ms, which makes timeout_ms: Some(),
         * set the timeout_ms for now to that.
         * If not, read the timeout_ms from config and set the
         * timeout_ms to that
         */
        match timeout_ms {
            Some(millis) => {
                self.timeout_ms = Duration::from_millis(millis);
                return Ok(self)
            },
            None => {
                self.timeout_ms = Config::read()?.timeout_ms();
                return Ok(self)
            }
        }
    }

    pub fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Result<Self, PolicyError> {
        /*
         * If user gives the working_dir, which makes working_dir: Some(),
         * set the working_dir for now to that.
         * If not, read the working_dir from config and set the
         * working_dir to that
         */
        match working_dir {
            Some(dir) => {
                self.working_dir = dir;
                return Ok(self)
            },
            None => {
                self.working_dir = Config::read()?.working_dir().clone();
                return Ok(self)
            }
        }
    }

    pub fn set_disable_timeout(mut self, disable: bool) -> Result<Self, PolicyError> {
        /*
         * If user gives the disable_timeout, which makes disable: true,
         * set the disable_timeout for now to that.
         * If not, read the disable_timeout from config and set the
         * disable_timeout to that
         */
        if !disable {
            self.disable_timeout = Config::read()?.disable_timeout();
            return Ok(self)
        }
        
        self.disable_timeout = disable;
        return Ok(self)
    }
}

