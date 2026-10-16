
use std::{fs, env};

use crate::{Result, Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
enum Project {
    Rust,
    Python,
    Javascript,
}

#[derive(Debug)]
pub struct Detector {
    project: Project,
    reason: String,
}

impl Detector {
    pub fn detect() -> Result<Self> {
        let paths = fs::read_dir(env::current_dir()?)?;
        for path in paths {
            let entry = path?;
            println!("{entry:#?}")
        }
        
        Ok (
            Self {
                project: Project::Rust,
                reason: "cargo.toml babyyy".to_string()
            }
        )
    }

    pub fn why(&self) -> Result<&str> {
        Ok(&self.reason)
    }
}

impl std::fmt::Display for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "Rust"),
            Self::Javascript => write!(f, "Javascript"),
            Self::Python => write!(f, "Python"),
        }
    }
}

