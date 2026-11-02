
use std::time::Duration;
use std::path::PathBuf;

use serde::{Serialize, Deserialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub(crate) disable_timeout: Option<bool>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) working_dir: Option<PathBuf>,
}

impl std::fmt::Display for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nPolicy")?;
        writeln!(f, "-------\n")?;
        if let Some(disable) = self.disable_timeout {
            writeln!(f, "Disable timeout? {}", disable)?;
        }

        if let Some(t_out) = self.timeout {
            writeln!(f, "Timeout {:?}", t_out)?;
        }

        if let Some(dir) = self.working_dir() {
            writeln!(f, "\nWorking directory: {:#?}\n", dir)?;
        }
        
        Ok(())
    }
}

///
/// Calling any method on policy changes data only in memory
/// so if you want to make the change persistent, aka change 
/// in disk, you MUST call the Config methods, not Policy ones
///
impl Policy {
    /// Just an alias for default()
    pub fn new() -> Self {
        Self::default()
    }


    pub fn timeout(&self) -> Option<&Duration> {
        self.timeout.as_ref()
    }
    
    pub fn disable_timeout(&self) -> Option<bool> {
        self.disable_timeout
    }
    
    pub fn working_dir(&self) -> Option<&PathBuf> {
        self.working_dir.as_ref()
    }

    ///
    /// If user gives the timeout, which makes timeout: Some(),
    /// set the timeout for now to that.
    /// If not, read the timeout from config and set the
    /// timeout to that
    ///
    pub fn set_timeout(&mut self, timeout: Duration) -> &Self {
        self.timeout = Some(timeout);
        self
    }

    ///
    /// If user gives the working_dir, which makes working_dir: Some(),
    /// set the working_dir for now to that.
    /// If not, read the working_dir from config and set the
    /// working_dir to that
    ///
    pub fn set_working_dir(&mut self, working_dir: PathBuf) -> &mut Self {
        self.working_dir = Some(working_dir);
        self
    }

    ///
    /// If user gives the disable_timeout, which makes disable: true,
    /// set the disable_timeout for now to that.
    /// If not, read the disable_timeout from config and set the
    /// disable_timeout to that
    ///
    pub fn set_disable_timeout(&mut self, disable: bool) -> &mut Self {
        self.disable_timeout = Some(disable);
        self
    }
}

