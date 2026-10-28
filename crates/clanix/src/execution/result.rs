
use std::path::{PathBuf, Path};
use std::borrow::Cow;

use uuid::Uuid;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

use crate::{Error, ResultError};
use crate::error::Result;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub enum ExecutionOutput {
    Separate {
        #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
        stdout: Vec<u8>,
        
        #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
        stderr: Vec<u8>,
    },
    Merged {
        #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
        output: Vec<u8>
    },
    Terminal,

    #[default]
    Unknown,
}


#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub(crate) id: Uuid,
    pub(crate) success: bool,
    pub(crate) status: Option<i32>,

    pub(crate) output: ExecutionOutput,

    pub(crate) working_dir: PathBuf,
}

fn bytes_to_str<S>(
    bytes: &[u8],
    serializer: S
    ) -> std::result::Result<S::Ok, S::Error> 
    where S: Serializer,
    {
        serializer.serialize_str(
            &String::from_utf8_lossy(bytes)
        )
    }

fn str_to_bytes<'de, D>(
    deserializer: D
    ) -> std::result::Result<Vec<u8>, D::Error> 
    where D: Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        Ok(string.into_bytes())
    }

impl std::fmt::Display for ExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nExecution Output")?;
        writeln!(f, "------------------\n")?;

        if self.is_separated() {
            if let Some(stdout_string) = &self.get_stdout_string() {
                if !stdout_string.trim().is_empty() {
                    writeln!(f, "\nStdout: ")?;
                    writeln!(f, "{}", stdout_string)?;
                }
            }
    
            if let Some(stderr_string) = &self.get_stderr_string() {
                if !stderr_string.trim().is_empty() {
                    writeln!(f, "\nStderr: ")?;
                    writeln!(f, "{}", stderr_string)?;
                }
            }
        } else if let Some(merged_output_string) = &self.get_merged_output_string() {
            if !merged_output_string.trim().is_empty() {
                writeln!(f, "\nOutput: ")?;
                writeln!(f, "{}", merged_output_string)?;
            }
        } else if self.is_terminal() {
            writeln!(f, "\n[TERMINAL/INTERACTIVE]")?;
        } else if self.is_unknown() {
            writeln!(f, "\n[UNKNOWN]")?;
        }

        Ok(())
    }
}
    
/// The format users see the output in the terminal
impl std::fmt::Display for ExecutionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nExecution Result")?;
        writeln!(f, "----------------\n")?;
        writeln!(f, "ID: {}", self.id)?;
        writeln!(f, "Success: {}", self.success)?;

        if let Some(status) = self.status {
            writeln!(f, "Status: {:?}", status)?;
        }
        
        writeln!(f, "\nWorking_dir: {}", self.working_dir.display())?;

        writeln!(f, "{}", self.output)?;
        
        Ok(())
    }
}

impl ExecutionOutput {
    pub fn is_separated(&self) -> bool {
        matches!(&self, Self::Separate {..})
    }

    pub fn is_merged(&self) -> bool {
        matches!(&self, Self::Merged{..})
    }

    pub fn is_terminal(&self) -> bool {
        matches!(&self, Self::Terminal)
    }

    pub fn is_unknown(&self) -> bool {
        matches!(&self, Self::Unknown)
    }

    pub fn get_merged_output(&self) -> Option<&[u8]> {
        match self {
            Self::Merged { output } => Some(output),
            _ => None
        }
    }

    pub fn get_merged_output_string(&self) -> Option<Cow<'_, str>> {
        self.get_merged_output().map(|output| String::from_utf8_lossy(output))
    }

    pub fn get_stdout(&self) -> Option<&[u8]> {
        // &self.stdout
        if let ExecutionOutput::Separate{stdout, ..} = &self {
            Some(&stdout)
        } else {
            None
        }
    }

    pub fn get_stdout_string(&self) -> Option<Cow<'_, str>> {
        // Not used anymore because of new structure
        // String::from_utf8_lossy(&self.stdout)
        // 
        // if let Some(stdout) = &self.get_stdout() {
        //     Some(String::from_utf8_lossy(&stdout))
        // } else {
        //     None
        // }

        // Equivalent to above if let Some() block
        self.get_stdout().map(|stdout| String::from_utf8_lossy(stdout))
    }

    pub fn get_stderr(&self) -> Option<&[u8]>  {
        // &self.stderr
        if let ExecutionOutput::Separate{stderr, ..} = &self {
            Some(&stderr)
        } else {
            None
        }
    }

    pub fn get_stderr_string(&self) -> Option<Cow<'_, str>> {
        // Not used anymore because of new structure
        // String::from_utf8_lossy(&self.stderr)
        
        self.get_stderr().map(|stderr| String::from_utf8_lossy(stderr))
    }
    
    pub fn set_stdout(&mut self, new_stdout: String) -> Result<&mut Self> {
        // self.stdout = new_stdout.into_bytes();
        // self

        if let ExecutionOutput::Separate{stdout, ..} = self {
            *stdout = new_stdout.into_bytes();
            Ok(self)
        } else {
            return Err(ResultError::SetStdoutError.into())
        }
    }

    pub fn set_stderr(&mut self, new_stderr: String) -> Result<&mut Self> {
        // self.stderr = new_stderr.into_bytes();
        // self

        if let ExecutionOutput::Separate{stderr, ..} = self {
            *stderr = new_stderr.into_bytes();
            Ok(self)
        } else {
            return Err(ResultError::SetStderrError.into())
        }
    }
}

impl ExecutionResult {
    /// These simple methods exist for the caller to call,
    /// because the internal structure might change and
    /// if the caller calls the internal elements directly,
    /// it could break,
    /// so calling these methods is safer because these will
    /// translate whatever was changed internally into what the
    /// callers expect; with no breaking changes

    /// We dont need this default because the Default trait automatically
    /// produces default values (eg: None for Option<>, 0 for i32 etc),
    /// and we eliminate the need to introduce the overhead of loading data
    /// onto the instance when it's just a default that's gonna be replaced
    /// anyway

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self)
            .map_err(|e| Error::Result(ResultError::JsonError(e)))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(&self)
            .map_err(|e| Error::Result(ResultError::TomlError(e)))
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

    pub fn output(&self) -> &ExecutionOutput {
        &self.output
    }

    pub fn is_output_separated(&self) -> bool {
        self.output.is_separated()
    }

    pub fn is_output_merged(&self) -> bool {
        self.output.is_merged()
    }

    pub fn is_output_terminal(&self) -> bool {
        self.output.is_terminal()
    }

    pub fn is_output_unknown(&self) -> bool {
        self.output.is_unknown()
    }
    
    pub fn get_stdout(&self) -> Option<&[u8]> {
        // &self.stdout
        self.output.get_stdout()
    }

    pub fn get_stdout_string(&self) -> Option<Cow<'_, str>> {
        // Not used anymore because of new structure
        // String::from_utf8_lossy(&self.stdout)
        
        self.output.get_stdout_string()
    }

    pub fn get_stderr(&self) -> Option<&[u8]>  {
        // &self.stderr

        self.output.get_stderr()
    }

    pub fn get_stderr_string(&self) -> Option<Cow<'_, str>> {
        // Not used anymore because of new structure
        // String::from_utf8_lossy(&self.stderr)
        
        self.output.get_stderr_string()
    }

    pub fn set_success(&mut self, new_success: bool) -> &mut Self {
        self.success = new_success;
        self
    }

    pub fn set_status(&mut self, new_status: Option<i32>) -> &mut Self {
        self.status = new_status;
        self
    }

    pub fn set_stdout(&mut self, new_stdout: String) -> Result<&mut Self> {
        // self.stdout = new_stdout.into_bytes();
        // self

        self.output.set_stdout(new_stdout)?;
        Ok(self)
    }

    pub fn set_stderr(&mut self, new_stderr: String) -> Result<&mut Self> {
        // self.stderr = new_stderr.into_bytes();
        // self

        self.output.set_stderr(new_stderr)?;
        Ok(self)
    }
}

