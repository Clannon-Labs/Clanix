
use std::{
    ffi::{OsStr,},
    fs,
    io::{self, Write},
    path::{Path, PathBuf,}
};
use std::collections::HashMap;

use walkdir::{DirEntry, WalkDir};

use crate::{Error, Result};

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
    // PathBuf is the file cuz of which the project was detected
    // to be what was reported
    detected: HashMap<Project, Vec<PathBuf>>,
}

fn take_input() -> Result<String> {
    let mut input = String::new();
    
    print!(">> Do you want it renamed properly? [y/N]: ");
    io::stdout().flush()?;

    io::stdin()
        .read_line(&mut input)
        .expect("Couldn't read your input!");

    Ok(input.to_ascii_lowercase())
}

fn is_hidden(entry: &DirEntry) -> bool {
    let name = entry.file_name();

    if name == OsStr::new(".") || name == OsStr::new("..") {
        return false;
    }

    Path::new(name)
        .as_os_str()
        .as_encoded_bytes()
        .starts_with(b".")
}

impl Detector {
    pub fn new() -> Self {
        Self {
            detected: HashMap::new()
        }
    }

    fn insert(&mut self, project: Project, reason: &Path) -> Result<PathBuf> {
        self.detected.entry(project)
            .or_insert_with(Vec::new)
            .push(reason.to_path_buf());

        Ok(reason.to_path_buf())
    }
    
    /// This just scans for the current directory 
    pub fn detect(&mut self) -> Result<()> {
        let walker = WalkDir::new(Path::new("."))
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !is_hidden(e));

        for entry in walker {
            match entry {
                Ok(dir_entry) => {
                    let path = dir_entry.path();
                    
                    if dir_entry.file_type().is_file() {
                        let Some(filename) = path.file_name() else {
                            return Err(
                                Error::InvalidFilename
                            )
                        };

                        if filename.eq_ignore_ascii_case("cargo.toml")
                            || filename.eq_ignore_ascii_case("cargo.lock") {

                                match filename.to_str() {
                                    Some(fname) if matches!(fname, "Cargo.toml" | "Cargo.lock") => {
                                        self.insert(Project::Rust, path)?;
                                    },
                                    _ => {
                                        eprintln!("\nWARNING: Non standard filename found {}!", path.display());
                                        
                                        if filename.eq_ignore_ascii_case("cargo.toml") {
                                            eprintln!("Rust tools often expect exactly 'Cargo.toml' !\n");
                                        } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                            eprintln!("Rust tools often expect exactly 'Cargo.lock' !\n");
                                        }

                                        match take_input()?.trim() {
                                            "y" | "yes" => {
                                                if filename.eq_ignore_ascii_case("cargo.toml") {
                                                    fs::rename(
                                                        path,
                                                        self.insert(Project::Rust, &path.with_file_name("Cargo.toml"))?
                                                    )?;
                                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                                    fs::rename(
                                                        path,
                                                        self.insert(Project::Rust, &path.with_file_name("Cargo.lock"))?
                                                    )?;
                                                }
                                            },
                                            _ => {
                                                if filename.eq_ignore_ascii_case("cargo.toml") {
                                                    self.insert(Project::Rust, path)?;
                                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                                    self.insert(Project::Rust, path)?;
                                                }
                                            },
                                        }

                                    }
                                }
                        } else if filename.eq_ignore_ascii_case("package.json") {

                            match filename.to_str() {
                                Some("package.json") => {
                                    self.insert(Project::Javascript, path)?;
                                },
                                _ => {
                                    eprintln!("\nWARNING: Non standard filename found {}!", path.display());
                                    eprintln!("JS/TS tools often expect exactly 'package.json' !\n");

                                    match take_input()?.trim() {
                                        "y" | "yes" => {
                                            fs::rename(
                                                path,
                                                self.insert(Project::Javascript, &path.with_file_name("package.json"))?
                                            )?;
                                        },
                                        _ => {
                                            self.insert(Project::Javascript, &path)?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                Err(e) => return Err(
                    Error::WalkError(e)
                )
            }
        }

        Ok(())
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

