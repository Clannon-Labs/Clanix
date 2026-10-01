
use serde::{Serialize, Deserialize};

use crate::{ExecutionRequest, ExecutionResult, Policy};

// Just a proposed idea for now, no real implementation yet

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEnvironment {
    request: ExecutionRequest,
    policy: Policy,
    result: ExecutionResult
}

