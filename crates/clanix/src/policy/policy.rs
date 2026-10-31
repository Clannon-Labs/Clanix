
use std::time::Duration;
use std::path::{PathBuf, Path};

use serde::{Serialize, Deserialize};

use crate::config::Config;
use crate::PolicyError;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub(crate) disable_timeout: bool,
    pub(crate) timeout: Duration,
    pub(crate) working_dir: PathBuf,
}

impl std::fmt::Display for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nPolicy")?;
        writeln!(f, "-------\n")?;
        writeln!(f, "Disable timeout? {}", self.disable_timeout)?;

        writeln!(f, "Timeout seconds: {:?}", self.timeout)?;

        writeln!(f, "\nWorking directory: {}\n", self.working_dir.display())?;
        
        Ok(())
    }
}

type Result<T> = std::result::Result<T, PolicyError>;

///
/// Calling any method on policy changes data only in memory
/// so if you want to make the change persistent, aka change 
/// in disk, you MUST call the Config methods, not Policy ones
///
impl Policy {
    pub fn default() -> Result<Self> {
        Ok(Self {
            disable_timeout: Config::default_disable_timeout(),
            timeout: Config::default_timeout(),
            working_dir: Config::default_working_dir()?,
        })
    }

    /// Just an alias for defaults()
    pub fn new() -> Result<Self> {
        Self::default()
    }


    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    
    pub fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }
    
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    ///
    /// If user gives the timeout, which makes timeout: Some(),
    /// set the timeout for now to that.
    /// If not, read the timeout from config and set the
    /// timeout to that
    ///
    pub fn set_timeout(&mut self, timeout: Duration) -> &Self {
        self.timeout = timeout;
        self
    }

    ///
    /// If user gives the working_dir, which makes working_dir: Some(),
    /// set the working_dir for now to that.
    /// If not, read the working_dir from config and set the
    /// working_dir to that
    ///
    pub fn set_working_dir(&mut self, working_dir: PathBuf) -> &mut Self {
        self.working_dir = working_dir;
        self
    }

    ///
    /// If user gives the disable_timeout, which makes disable: true,
    /// set the disable_timeout for now to that.
    /// If not, read the disable_timeout from config and set the
    /// disable_timeout to that
    ///
    pub fn set_disable_timeout(&mut self, disable: bool) -> &mut Self {
        if disable {
            self.disable_timeout = disable
        }

        self
    }
}

