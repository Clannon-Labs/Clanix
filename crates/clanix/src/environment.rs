
use std::ffi::OsString;

use serde::Serialize;

use crate::{Config, ExecutionRequest, ExecutionResult};
use crate::Policy;
use crate::ConfigBuilder;
use crate::{Error, Result};

use crate::ResultError;

// This could be made richer and better instead of just a glue of other structs
#[derive(Debug, Clone, Serialize)]
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
    pub fn new_request<S: Into<OsString>>(&self, program: S) -> Result<ExecutionRequest> {
        Ok(ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default()?,
        })
    }

    /// After creating an execution request (which is mandatory),
    /// execute it
    pub fn execute_request(&mut self, request: ExecutionRequest) -> Result<()> {
        let result = ExecutionRequest::execute(&request)?;
        self.request = request;
        self.result = Some(result);

        Ok(())
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
    pub fn config_mut(&mut self) -> Result<&mut ConfigBuilder> {
        Ok(&mut self.config)
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
