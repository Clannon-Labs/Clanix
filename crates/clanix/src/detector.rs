
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

#[derive(Debug, Hash, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Repository {
    Git,
    Unknown,
}

#[derive(Debug)]
pub struct Detector {
    // PathBuf is the file cuz of which the project was detected
    // to be what was reported
    project: HashMap<Project, Vec<PathBuf>>,
    repository: HashMap<Repository, Vec<PathBuf>>,
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
            project: HashMap::new(),
            repository: HashMap::new()
        }
    }

    /// Get the project, if the values ar already present, append to them,
    /// if it's empty, create a new vector and push the value
    fn insert_project(&mut self, project: Project, reason: &Path) -> Result<PathBuf> {
        self.project.entry(project)
            .or_insert_with(Vec::new)
            .push(reason.to_path_buf());

        Ok(reason.to_path_buf())
    }

    /// Same for repository
    fn insert_repository(&mut self, repo: Repository, reason: &Path) -> Result<PathBuf> {
        self.repository.entry(repo)
            .or_insert_with(Vec::new)
            .push(reason.to_path_buf());

        Ok(reason.to_path_buf())
    }
    
    /// This scans the current directory and its children
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
                                Error::InvalidFilename(path.to_path_buf())
                            )
                        };

                        if filename.eq_ignore_ascii_case("cargo.toml")
                            || filename.eq_ignore_ascii_case("cargo.lock") {

                                match filename.to_str() {
                                    Some(fname) if matches!(fname, "Cargo.toml" | "Cargo.lock") => {
                                        self.insert_project(Project::Rust, path)?;
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
                                                        self.insert_project(Project::Rust, &path.with_file_name("Cargo.toml"))?
                                                    )?;
                                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                                    fs::rename(
                                                        path,
                                                        self.insert_project(Project::Rust, &path.with_file_name("Cargo.lock"))?
                                                    )?;
                                                }
                                            },
                                            _ => {
                                                if filename.eq_ignore_ascii_case("cargo.toml") {
                                                    self.insert_project(Project::Rust, path)?;
                                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                                    self.insert_project(Project::Rust, path)?;
                                                }
                                            },
                                        }

                                    }
                                }
                        } else if filename.eq_ignore_ascii_case("package.json") {

                            match filename.to_str() {
                                Some("package.json") => {
                                    self.insert_project(Project::Javascript, path)?;
                                },
                                _ => {
                                    eprintln!("\nWARNING: Non standard filename found {}!", path.display());
                                    eprintln!("JS/TS tools often expect exactly 'package.json' !\n");

                                    match take_input()?.trim() {
                                        "y" | "yes" => {
                                            fs::rename(
                                                path,
                                                self.insert_project(Project::Javascript, &path.with_file_name("package.json"))?
                                            )?;
                                        },
                                        _ => {
                                            self.insert_project(Project::Javascript, &path)?;
                                        }
                                    }
                                }
                            }
                        }
                    } else if dir_entry.file_type().is_dir() {
                        let dir_name = match path.file_name() {
                          Some(name) => name,
                          None => {
                              if path == Path::new(".") {
                                  OsStr::new(".")
                              } else {
                                  return Err(
                                      Error::InvalidDirectoryName(path.to_path_buf())
                                  )
                              }
                          }  
                        };

                        if dir_name.eq_ignore_ascii_case(".git") {
                            match dir_name.to_str() {
                                Some(".git") => {
                                    self.insert_repository(Repository::Git, path)?;
                                },
                                _ => {
                                    eprintln!("\nWARNING: Non standard directory found {}!", path.display());
                                    eprintln!("Other tools expect exactly '.git' !\n");

                                    match take_input()?.trim() {
                                        "y" | "yes" => {
                                            fs::rename(
                                                path,
                                               self.insert_repository(Repository::Git, &path.with_file_name(".git"))? 
                                            )?;
                                        },
                                        _ => {
                                            self.insert_repository(Repository::Git, path)?;
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

        for key in self.project.keys() {
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

impl std::fmt::Display for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git => write!(f, "Git"),
            _ => write!(f, "Unknown")
        }
    }
}

impl std::fmt::Display for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\n====Detection Report====")?;

        for (project, reasons) in &self.project {
            writeln!(f, "\nProject: {project}")?;
            writeln!(f, "Reasons:")?;

            for reason in reasons {
                writeln!(f, " - {}", reason.display())?;
            }
        }

        writeln!(f, "\n========================")
    }
}
