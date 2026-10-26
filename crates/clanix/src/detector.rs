
use std::{env, fs, io::{self, Write}};
use std::collections::HashMap;

use crate::Result;

#[derive(Debug, Hash, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Project {
    Rust,
    Python,
    Javascript,
    Unknown,
}

#[derive(Debug)]
pub struct Detector {
    // String is the reason cuz of which the project was detected
    detected: HashMap<Project, Vec<String>>,
}

fn take_input() -> Result<String> {
    let mut input = String::new();
    
    print!("Do you want it/them renamed properly? [y/N]: ");
    io::stdout().flush()?;

    io::stdin()
        .read_line(&mut input)
        .expect("Couldn't read your input!");

    Ok(input.to_ascii_lowercase())
}

impl Detector {
    fn new() -> Self {
        Self {
            detected: HashMap::new()
        }
    }

    fn insert(&mut self, project: Project, reason: &str) -> Result<String> {
        self.detected.entry(project)
            .or_insert_with(Vec::new)
            .push(reason.to_string());

        Ok(reason.to_string())
    }

    pub fn detect() -> Result<Self> {
        let entries: Vec<std::result::Result<fs::DirEntry, std::io::Error>> = 
            fs::read_dir(env::current_dir()?)?.collect();

        let mut detected = Self::new();
        
        for path in entries {
            let entry = path?;
            let filename = entry.file_name();
            
            if filename.eq_ignore_ascii_case("cargo.toml") || 
            filename.eq_ignore_ascii_case("cargo.lock") {

                match filename.to_str() {
                    Some(fname) if matches!(fname, "Cargo.toml" | "Cargo.lock") => {
                        detected.insert(Project::Rust, &fname)?;
                        
                    },
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
                                    // The reason for putting detected.insert() inside rename
                                    // is to not accidently insert a different filename and rename
                                    // a different filename
                                    fs::rename(&filename, 
                                        detected.insert(Project::Rust, "Cargo.toml")?
                                    )?;
                                    
                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                    fs::rename(&filename, 
                                        detected.insert(Project::Rust, "Cargo.lock")?
                                    )?;
                                }
                            },
                            "n" | "no" => {
                                detected.insert(Project::Rust, &filename.to_string_lossy())?;
                            },
                            _ => eprintln!("Invalid input!")
                        }
                        
                    }
                }
                                
            } else if filename.eq_ignore_ascii_case("package.json") {

                match filename.to_str() {
                    Some(fname) if matches!(fname, "package.json") => {
                        detected.insert(Project::Javascript, fname)?;
                    },
                    _ => {
                        eprintln!("\nWarning: Non-standard filename found ({:?}).\n",filename);
                        eprintln!("Javascript/Typescript tools expect exactly 'package.json'\n");
                        eprintln!("You are advised to rename them correctly\n");


                        match take_input()?.trim() {
                            "y" | "yes" => {
                                if filename.eq_ignore_ascii_case("package.json") {
                                    fs::rename(&filename, 
                                        detected.insert(Project::Javascript, "package.json")?
                                    )?;
                                }
                            },
                            "n" | "no" => {
                                detected.insert(Project::Javascript, &filename.to_string_lossy())?;
                            },
                            _ => eprintln!("Invalid input!")
                        }
                    }
                }
            }
        }
        
        Ok(detected)
        
    }
    
    pub fn project(&self) -> Result<Box<[&Project]>> {
        let mut project = Vec::new();

        for key in self.detected.keys() {
            project.push(key)
        }

        Ok(project.into_boxed_slice())
    }
    
    pub fn why(&self) -> Result<&Self> {
        Ok(&self)
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

