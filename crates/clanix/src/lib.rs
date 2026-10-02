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
mod error;
mod execution;
mod policy;
mod environment;
// mod stages;

// Custom crates
pub use policy::*;
pub use config::*;
pub use execution::*;
pub use environment::*;

pub use error::*;
// pub use stages::*;
