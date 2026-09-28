//! The PolicyTrait is implemented by any struct that does
//! the policy/config related work.
//! 
//! Just like the Policy and Config/ConfigBuilder structs, which handle the policies
//! so they implement this trait to get the default values.
//! 
//! And if they want the default values, they must also implement the methods to change
//! those values (not the default ones) in memory or on disk depending on what their
//! role is

use std::env;
use std::path::PathBuf;
use std::time::Duration;

pub trait PolicyTrait {
    fn defaults() -> Self;

    fn timeout_ms(&self) -> Duration;
    fn disable_timeout(&self) -> bool;
    fn working_dir(&self) -> &PathBuf;
    
    fn set_timeout_ms(self, timeout_ms: Option<u64>) -> Self;
    fn set_working_dir(self, working_dir: Option<PathBuf>) -> Self;

    /// Default timeout_ms of 1ms is just for experimentation
    /// and will be changed in future so it's more realistic
    fn default_timeout_ms() -> Duration {
        Duration::from_millis(1)
    }
    
    fn default_working_dir() -> PathBuf {
        env::current_dir().unwrap() // replace this .unwrap() later
    }

    /// timeout is enabled (or not disabled) by default
    fn default_disable_timeout() -> bool {
        false 
    }
}
