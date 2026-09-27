
use std::env;
use std::path::PathBuf;
use std::time::Duration;

pub trait PolicyTrait {
    fn defaults() -> Self;
    fn new() -> Self 
        where Self: Sized {
        Self::defaults()
    }
    
    fn default_timeout_ms() -> Duration {
        Duration::from_millis(1)
    }
    
    fn default_working_dir() -> PathBuf {
        env::current_dir().unwrap() // replace this .unwrap() later
    }
    
    fn default_disable_timeout() -> bool {
        false // timeout is enabled (or not disabled) by default
    }

    fn timeout_ms(&self) -> Duration;
    fn disable_timeout(&self) -> bool;
    fn working_dir(&self) -> &PathBuf;
    
    fn set_timeout_ms(self, timeout_ms: Option<u64>) -> Self;
    fn set_disable_timeout(self, disable: bool) -> Self;
    fn set_working_dir(self, working_dir: Option<PathBuf>) -> Self;
}
