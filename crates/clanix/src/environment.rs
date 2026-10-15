
use std::ffi::OsString;

use serde::Serialize;

use crate::{Config, ExecutionRequest, ExecutionResult};
use crate::Policy;
use crate::ConfigBuilder;
use crate::{Error, Result}; // get the default Error type automatically

use crate::ResultError;

// This could be made richer and better instead of just a glue of other structs
#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    request: ExecutionRequest,
    result: Option<ExecutionResult>,
    config: ConfigBuilder,
}

impl Environment {
    pub fn new() -> Result<Self> {
        Ok(Self {
            request: ExecutionRequest::default()?,
            result: None,
            config: ConfigBuilder::default()?,
        })
    }

    pub fn new_request<S: Into<OsString>>(&self, program: S) -> Result<ExecutionRequest> {
        Ok(ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default()?,
        })
    }

    pub fn execute_request(&mut self, request: ExecutionRequest) -> Result<()> {
        let result = ExecutionRequest::execute(&request)?;
        self.request = request;
        self.result = Some(result);

        Ok(())
    }
    
    pub fn new_result(&self) -> Result<ExecutionResult> {
        Ok(ExecutionResult::default())
    }

    pub fn set_default_config(&mut self) -> Result<&ConfigBuilder> {
        let config = Config::set_default()?;
        self.config = config;
        Ok(&self.config)
        
    }
    
    pub fn what(&self) -> Result<&Self> {
        Ok(&self)
    }

    pub fn request(&self) -> Result<&ExecutionRequest> {
        Ok(&self.request)
    }

    pub fn result(&self) -> Result<&ExecutionResult> {
        self.result
            .as_ref()
            .ok_or_else(|| Error::Result(ResultError::Empty)) 
    }

    pub fn result_mut(&mut self) -> Result<&mut ExecutionResult> {
        self.result
            .as_mut()
            .ok_or_else(|| Error::Result(ResultError::Empty))
            
    }
    
    pub fn config(&self) -> Result<&ConfigBuilder> {
        Ok(&self.config)
    }

    pub fn config_mut(&mut self) -> Result<&mut ConfigBuilder> {
        Ok(&mut self.config)
    }

    pub fn policy(&self) -> Result<&Policy> {
        Ok(&self.request.policy)
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
