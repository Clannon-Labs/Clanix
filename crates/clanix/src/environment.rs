
//! In the clanix crate, this is the entry point for its consumers to use clanix
//! They can't directly access anything inside clanix directly

use std::ffi::OsString;

use serde::Serialize;

use crate::{ExecutionRequest, ExecutionResult};
use crate::Policy;
use crate::ConfigBuilder;
use crate::Error;

// Just a proposed idea for now, no real implementation yet

// This could be made richer and better instead of just a glue of other structs
#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    request: ExecutionRequest,
    // policy: Policy, // Policy is already in the request, so we dont need to replicate it
    result: Option<ExecutionResult>
}

impl Environment {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            request: ExecutionRequest::default()?,
            result: None
        })
    }

    pub fn request<S: Into<OsString>>(program: S) -> Result<ExecutionRequest, Error> {
        Ok(ExecutionRequest {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::defaults()?
        })
    }

    pub fn result() -> Result<ExecutionResult, Error> {
        ExecutionResult::defaults()
            .map_err(Error::from)
    }

    pub fn new_config() -> Result<ConfigBuilder, Error> {
        Ok(ConfigBuilder::new()?)
    }

    pub fn set_default_config() -> Result<ConfigBuilder, Error> {
        Ok(ConfigBuilder::set_default()?)
    }

    pub fn read_config() -> Result<ConfigBuilder, Error> {
        Ok(ConfigBuilder::read()?)
    }
}
