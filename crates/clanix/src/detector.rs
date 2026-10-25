
use std::{io::{self, Write}, fs, env};

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

fn take_input() -> Result<String> {
    let mut input = String::new();
    
    print!("Do you want it/them renamed properly? [y/N]: ");
    io::stdout().flush()?;

    io::stdin()
        .read_line(&mut input)
        .expect("Couldn't read your input!");

    Ok(input)
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
                        eprintln!("\nWarning: Non-standard filename found ({:?}).\n",filename);
                        eprintln!("Rust tools expect exactly 'Cargo.toml' or 'Cargo.lock'\n");
                        eprintln!("You are advised to rename them correctly\n");

                        
                        match take_input()?.trim() {
                            "y" | "yes" => {
                                // If we reached this point, we are sure that the filenames
                                // are not the standard ones, so we can say for sure that 
                                // 
                                // we are not wasting our time trying to rename the file that's
                                // already in the standard form
                                if filename.eq_ignore_ascii_case("cargo.toml") {
                                    fs::rename(&filename, "Cargo.toml")?
                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                    fs::rename(&filename, "Cargo.lock")?
                                }
                            },
                            "n" | "no" => {},
                            _ => eprintln!("Invalid input!")
                        }
                        
                    }
                }
                
                return Ok (
                    Self {
                        project: Project::Rust,
                        reason: format!("\nCuz of {filename:#?}\n")
                    }
                )
            } else if filename.eq_ignore_ascii_case("package.json") {

                match filename.to_str() {
                    Some("package.json") => {},
                    _ => {
                        eprintln!("\nWarning: Non-standard filename found ({:?}).\n",filename);
                        eprintln!("Javascript/Typescript tools expect exactly 'package.json'\n");
                        eprintln!("You are advised to rename them correctly\n");

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

