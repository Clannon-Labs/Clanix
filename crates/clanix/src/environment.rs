
use std::ffi::OsString;

use serde::Serialize;

use crate::{Config, ExecutionEvent, ExecutionRequest, ExecutionResult};
use crate::Policy;
use crate::ConfigBuilder;
use crate::{Error, Result};

use crate::ResultError;

/// The main Environment struct that provides a clean set of APIs the users can
/// call to manage the environment more easily instead of having to think about every
/// minor detail
/// 
/// But note that you can still call the methods provided by each layer (i.e. ExecutionRequest,
/// ExecutionResult, Config etc); only if you want finer controls on the environment ofc
/// 
/// If you just want to use the environment without having to coordinate between multiple stages
/// and managing each thing, the Environment will increasingly provide a better set of APIs
/// to do so
/// 
#[derive(Debug, Clone, Serialize)] // This could be made more richer and informative ig
pub struct Environment {
    request: ExecutionRequest,
    result: Option<ExecutionResult>,
    config: ConfigBuilder,
}

impl Environment {
    /// Create a brand new environment instance with the default values
    pub fn new() -> Result<Self> {
        Ok(Self {
            request: ExecutionRequest::default()?,
            result: None,
            config: ConfigBuilder::default()?,
        })
    }

    /// Create a new execution request instance.
    /// 
    /// Note that this just creates an instance of execution request,
    /// It doesn't actualy execute the request
    /// 
    /// It lets you get the ExecutionRequest instance the moment you call it
    /// and pass the arguments to it along the way (i.e. by chaining commands)
    /// and at last, when you want to actually execute it, you can call the
    /// executer method
    pub fn new_request<S: Into<OsString>>(&self, program: S) -> Result<ExecutionRequest> {
        Ok(ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default()?,
        })
    }

    /// After creating an execution request (which is mandatory),
    /// execute it
    /// 
    /// Note that this function immediately stores the request in the environment
    /// before it executes it and executes the request from the request stored in 
    /// the environment
    /// 
    /// This doesn't seem so different but if we were executing the request before storing 
    /// to environment, then incase the execution fails, the environment would hold the older
    /// request and result, which might make the clients think that the older request has failed
    pub fn execute_request(&mut self, request: ExecutionRequest) -> Result<&ExecutionResult> {
        self.request = request;
        let result = self.request.execute()?;
        self.result = Some(result);

        self.result()
    }

    /// This method is also the same as the above, it too stores the request before it executes it
    /// 
    /// Since these methods are just wrappers around the actual executer methods, their behaviour is
    /// identical
    pub fn execute_request_with_events<F>(
        &mut self, 
        request: ExecutionRequest,
        on_event: F
    ) -> Result<&ExecutionResult> 
        where 
            F: FnMut(ExecutionEvent)
    {
        self.request = request;
        let result = self.request.execute_with_events(on_event)?;
        self.result = Some(result);

        self.result()
    }

    /// Get the default result instance
    /// 
    /// Note that this is just for the case where you just want an execution result
    /// instance without actually running anything.
    /// It's not the result you get after executing the request
    pub fn new_result(&self) -> ExecutionResult {
        ExecutionResult::default()
    }

    /// Set the config to default
    pub fn set_default_config(&mut self) -> Result<&ConfigBuilder> {
        let config = Config::set_default()?;
        self.config = config;
        Ok(&self.config)
        
    }

    /// Get the reference of request field out of the environment instance
    pub fn request(&self) -> &ExecutionRequest {
        &self.request
    }

    /// Get the reference of result field out of the environment instance
    pub fn result(&self) -> Result<&ExecutionResult> {
        self.result
            .as_ref()
            .ok_or_else(|| Error::Result(ResultError::Empty)) 
    }

    /// Get the mutable reference of the result field
    pub fn result_mut(&mut self) -> Result<&mut ExecutionResult> {
        self.result
            .as_mut()
            .ok_or_else(|| Error::Result(ResultError::Empty))
            
    }

    /// Get the reference of config/configbuilder instance
    pub fn config(&self) -> &ConfigBuilder {
        &self.config
    }

    /// Get the mutable reference of config/configbuilder instance
    pub fn config_mut(&mut self) -> &mut ConfigBuilder {
        &mut self.config
    }

    /// Get the reference of config instance that was passed when the
    /// user made the execution request
    /// 
    /// Note that policy is the temporary (one for each run) config that
    /// gets reset for each run
    /// 
    /// The persistent version is the config, which stays on disk
    /// and can stay same for all runs
    pub fn policy(&self) -> &Policy {
        &self.request.policy
    }

}

impl Environment {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Error::from)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(Error::from)
    }
}


impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nENVIRONMENT")?;
        writeln!(f, "=============")?;
        writeln!(f, "{}", self.request)?;

        // Print result only if it has some value,
        // if not, print None.
        // 
        // We used this check to avoid printing it as Some()
        // in the terminal
        if let Some(result) = &self.result {
            writeln!(f, "{}", result)?;
        } else {
            writeln!(f, "Result: None")?;
        }
        
        Ok(())
    }
}
