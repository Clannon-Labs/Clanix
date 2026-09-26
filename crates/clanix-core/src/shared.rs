
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
        Duration::from_millis(100)
    }
    
    fn default_working_dir() -> PathBuf {
        env::current_dir().unwrap() // replace this .unwrap() later
    }
    
    fn default_timeout_disable() -> bool {
        false
    }
    
    fn set_timeout_ms(self, timeout_ms: Option<u64>) -> Self;
    // fn set_timeout_disable(self, timeout_disable: Option<bool>) -> Self;
    fn set_working_dir(self, working_dir: Option<PathBuf>) -> Self;
}
