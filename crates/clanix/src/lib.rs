//! clanix is the core library that does the actual job behind the scenes.
//!  
//! Policy is the in-memory config of the rules/policies we set for the processes
//! and Config is the persistent version.
//! 
//! ExecutionRequest implements the builder pattern so it can provide the default values
//! of the struct as rust doesn't allow us to do it with the same struct.
//! This gives clients a clean and readable API instead of having to handle complex
//! logic.
//! 
//! This also keeps the internal messy fields private and give an easy way for clients to 
//! handle the instance.
//! 
//! They can set the fields they want and leave others as is.
//! 
//! This also allows clients to chain the methods on struct instances and allow them to
//! first set the values they want by chaining the methods and using a final method to 
//! actually use the data to do some operation

mod config;
mod shared;
mod execution;

use std::path::PathBuf;
use std::time::Duration;

use serde::{Serialize, Deserialize};

// Custom crates
pub use config::Config; // Export so it is usable from outside + use it here
pub use shared::PolicyTrait;
pub use execution::*;

// use crate::config::ConfigBuilder; // Export + use it here
   
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    disable_timeout: bool,
    timeout_ms: Duration,
    working_dir: PathBuf,
}

/*
 * Calling any method on policy changes data only in memory
 * so if you want to make the change persistent, aka change 
 * in disk, you MUST call the Config methods, not Policy ones
 */
impl PolicyTrait for Policy {
    fn defaults() -> Self {
        Self {
            disable_timeout: Self::default_disable_timeout(),
            timeout_ms: Self::default_timeout_ms(),
            working_dir: Self::default_working_dir(),
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
        /*
         * If user gives the timeout_ms, which makes timeout_ms: Some(),
         * set the timeout_ms for now to that.
         * If not, read the timeout_ms from config and set the
         * timeout_ms to that
         */
        match timeout_ms {
            Some(millis) => {
                self.timeout_ms = Duration::from_millis(millis);
                return self
            },
            None => {
                self.timeout_ms = Config::read().timeout_ms();
                return self
            }
        }
    }

    fn set_working_dir(mut self, working_dir: Option<PathBuf>) -> Self {
        /*
         * If user gives the working_dir, which makes working_dir: Some(),
         * set the working_dir for now to that.
         * If not, read the working_dir from config and set the
         * working_dir to that
         */
        match working_dir {
            Some(dir) => {
                self.working_dir = dir;
                return self
            },
            None => {
                self.working_dir = Config::read().working_dir().clone();
                return self
            }
        }
    }
}

/*
 * User would call the methods from Config to interact
 * with the config file on disk
 * 
 * But if they called the methods from Policy, they will
 * be interacting with data in memory (not on disk)
 */

impl Policy {
    // Maybe giving defaults when new() is called isn't quite goood?
    // But I am yet to figure this out incase of Policy
    pub fn new() -> Self {
        Self::defaults()
    }
    
    pub fn set_disable_timeout(mut self, disable: bool) -> Self {
        /*
         * If user gives the disable_timeout, which makes disable: true,
         * set the disable_timeout for now to that.
         * If not, read the disable_timeout from config and set the
         * disable_timeout to that
         */
        if !disable {
            self.disable_timeout = Config::read().disable_timeout();
            return self
        }
        
        self.disable_timeout = disable;
        return self
    }
}
