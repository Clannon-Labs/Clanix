
use std::{fs, env};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Project {
    Rust,
    Python,
    Javascript,
    Unknown,
}

#[derive(Debug)]
pub struct Detector {
    project: Project,
    reason: String,
}

impl Detector {
    pub fn detect() -> Result<Self> {
        let entries: Vec<std::result::Result<fs::DirEntry, std::io::Error>> = 
            fs::read_dir(env::current_dir()?)?.collect();
        
        let count = entries.len();

        let mut iterator = 1;
        for path in entries {
            let entry = path?;
            let filename = entry.file_name();
            
            if filename.eq_ignore_ascii_case("cargo.toml") || 
            filename.eq_ignore_ascii_case("cargo.lock") {

                match filename.to_str() {
                    Some("Cargo.toml" | "Cargo.lock") => {},
                    _ => {
                        eprintln!(
                            "\nWarning: Non-standard filename found ({:?}).\nRust tools expect exactly 'Cargo.toml' or 'Cargo.lock'\n",
                            filename
                        )
                    }
                }
                
                return Ok (
                    Self {
                        project: Project::Rust,
                        reason: format!("Cuz of {filename:#?}")
                    }
                )
            } else if filename.eq_ignore_ascii_case("package.json") {

                match filename.to_str() {
                    Some("package.json") => {},
                    _ => {
                        eprintln!(
                            "\nWarning: Non-standard filename found ({:?}).\nJavascript/Typescript tools expect exactly 'package.json'",
                            filename
                        )
                    }
                }
                
                return Ok (
                    Self {
                        project: Project::Javascript,
                        reason: format!("Cuz of {filename:#?}")
                    }
                )
            } else {
                if iterator < count {
                    iterator += 1;
                    continue
                } 
            }
        }
        
        Ok (
            Self {
                project: Project::Unknown,
                reason: format!("Unknown")
            }
        )
        
    }

    pub fn project(&self) -> Result<&Project> {
        Ok(&self.project)
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
            _ => write!(f, "Unknown")
        }
    }
}

