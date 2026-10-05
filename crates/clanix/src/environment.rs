
use std::ffi::OsString;

use serde::Serialize;

use crate::{ExecutionRequest, ExecutionResult};
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

    pub fn execute_request(mut self, request: ExecutionRequest) -> Result<Self> {
        let result = ExecutionRequest::execute(&request)?;
        self.request = request;
        self.result = Some(result);

        Ok(self)
    }
    
    pub fn new_result(&self) -> Result<ExecutionResult> {
        Ok(ExecutionResult::default())
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
    
    pub fn config(&self) -> Result<&ConfigBuilder> {
        Ok(&self.config)
    }

    pub fn policy(&self) -> Result<&Policy> {
        Ok(&self.request.policy)
    }

}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ENVIRONMENT")?;
        writeln!(f, "============")?;
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
