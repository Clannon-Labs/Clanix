
use std::path::{PathBuf, Path};
use std::borrow::Cow;

use uuid::Uuid;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

use crate::Policy;
use crate::ResultError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub(crate) id: Uuid,
    pub(crate) success: bool,
    pub(crate) status: Option<i32>,

    #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
    pub(crate) stdout: Vec<u8>,

    #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
    pub(crate) stderr: Vec<u8>,  
  
    pub(crate) working_dir: PathBuf,
}

fn bytes_to_str<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> 
    where S: Serializer,
    {
        serializer.serialize_str(
            &String::from_utf8_lossy(bytes)
        )
    }

fn str_to_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error> 
    where D: Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        Ok(string.into_bytes())
    }


impl ExecutionResult {
    // These simple methods exist for the caller to call,
    // because the internal structure might change and
    // if the caller calls the internal elements directly,
    // it could break,
    // so calling these methods is safer because these will
    // translate whatever was changed internally into what the
    // callers expect; with no breaking changes

    pub fn defaults() -> Result<Self, ResultError> {
        /*
            * If user/client gave working dir, then use it for this run,
            * if not, read the working dir set in config and put that as default 
            * for this run
            */
        
        Ok(Self {
            id: Uuid::new_v4(),
            success: false,
            status: None,
            stdout: vec![],
            stderr: vec![],
            working_dir: Policy::defaults()
                .map_err(|_| ResultError::InvalidWorkingDir)?
                .working_dir()
                .to_path_buf()
        })
    }

    pub fn id(&self) -> &Uuid {
        &self.id
    }

    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }
    
    pub fn success(&self) -> bool {
        self.success
    }
    
    pub fn status(&self) -> Option<i32> {
        self.status
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stdout_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub fn stderr(&self) -> &[u8]  {
        &self.stderr
    }

    pub fn stderr_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    pub fn set_success(&mut self, new_success: bool) {
        self.success = new_success
    }

    pub fn set_status(&mut self, new_status: Option<i32>) {
        self.status = new_status
    }
    
    pub fn set_stdout(&mut self, new_stdout: String) {
        self.stdout = new_stdout.into_bytes()
    }

    pub fn set_stderr(&mut self, new_stderr: String) {
        self.stderr = new_stderr.into_bytes()
    }
}

