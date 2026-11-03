use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{Config, Policy, ResolveError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Effective {
    timeout: Duration,
    disable_timeout: bool,
    working_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resolver {
    config: Config,
}

type Result<T> = std::result::Result<T, ResolveError>;

impl Effective {
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn disable_timeout(&self) -> bool {
        self.disable_timeout
    }

    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }
}

impl Resolver {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn from_disk() -> Result<Self> {
        Ok(Self {
            config: Config::read()?,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// How it resolves the values from policy/config:
    ///
    /// It first looks at the policy to see if it has any value or not
    /// i.e. timeout/working_dir etc;
    ///
    /// If it does, it uses that and ignores
    /// the value for the same field we have might have in config.
    ///
    /// It it doesn't however, it takes that value from the config;
    /// which is guaranteed to give us the value for that field,
    /// except in the case of working_dir that needs to be calculated
    /// at the runtime.
    ///
    /// After getting all the fields required to be present in the Effective
    /// type, it returns the Effective type
    pub fn resolve(&self, policy: &Policy) -> Result<Effective> {
        let config = self.config();

        Ok(Effective {
            timeout: match policy.timeout() {
                Some(time) => *time,
                None => config.timeout(),
            },
            disable_timeout: match policy.disable_timeout() {
                Some(dis) => dis,
                None => config.disable_timeout(),
            },
            working_dir: match policy.working_dir() {
                Some(dir) => dir.clone(),
                None => match config.working_dir() {
                    Some(dir) => dir.clone(),
                    None => std::env::current_dir().map_err(ResolveError::WorkingDir)?,
                },
            },
        })
    }
}

