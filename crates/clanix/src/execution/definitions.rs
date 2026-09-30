
use std::path::PathBuf;
use std::ffi::OsString;

use uuid::Uuid;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

use crate::Policy;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub(super) program: OsString,
    pub(super) args: Vec<OsString>,
    pub(super) policy: Policy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequestBuilder {
    pub(super) program: OsString,
    pub(super) args: Vec<OsString>,
    pub(super) policy: Policy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub(super) id: Uuid,
    pub(super) success: bool,
    pub(super) status: Option<i32>,

    #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
    pub(super) stdout: Vec<u8>,

    #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
    pub(super) stderr: Vec<u8>,
    
    pub(super) working_dir: PathBuf,
}

fn bytes_to_str<S>(bytes: &[u8], serializer: S) -> Result<S:: Ok, S::Error>
    where S: Serializer,
    {
        serializer.serialize_str(
            &String::from_utf8_lossy(bytes)
        )
    }

fn str_to_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.into_bytes())
    }
