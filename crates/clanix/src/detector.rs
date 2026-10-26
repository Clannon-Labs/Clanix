
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

#[derive(Debug, Hash, Clone, PartialEq, Eq)]
struct Reason {
    // PathBuf is the file cuz of which the project/repository was detected
    // to be what was reported
    // 
    confidence: u64,
    evidence: Vec<Evidence>,
}

#[derive(Debug, Hash, Clone, PartialEq, Eq)]
struct Evidence {
    file: PathBuf,
    text: Option<String>,
}

#[derive(Debug)]
pub struct Detector {
    project: HashMap<Project, Reason>,
    repository: HashMap<Repository, Reason>,
}

fn take_input() -> Result<String> {
    let mut input = String::new();
    
    print!(">> Do you want it renamed properly? [y/N]: ");
    io::stdout().flush()?;

    io::stdin()
        .read_line(&mut input)?;

    Ok(input.to_ascii_lowercase())
}

fn should_exclude(entry: &DirEntry) -> bool {
    // if name == OsStr::new(".") || name == OsStr::new("..") {
    //     return false;
    // }

    // .git written in any case is not excluded!
    let is_git = entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(".git"));
    
    if is_git {
        return false;
    }

    // To prevent going inside the git directory
    let is_inside_git = entry
        .path()
        .ancestors()
        .skip(1)
        .any(|ancestor| {
            ancestor
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
        });

    if is_inside_git {
        return true;
    }
    
    if matches!(entry.file_name().to_str(),
        Some(".") | // current/root directory, so shouldn't be excluded
        Some("..") // Paren directory
    ) {
        return false;
    }

    entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.starts_with('.'))
}

impl Reason {
    fn new() -> Self {
        Self {
            confidence: 0,
            evidence: Vec::new(),
        }
    }
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
    fn insert_project(
        &mut self,
        project: Project,
        path: &Path,
        text: Option<String>
    ) {

        let reason = self.project
            .entry(project)
            // This gives a &mut Reason
            .or_insert_with(Reason::new);
        
        reason.confidence += 1;
        reason.evidence.push(Evidence {
            file: path.to_path_buf(),
            text,
        })
    }

    /// Same for repository
    fn insert_repository(
        &mut self, 
        repo: Repository, 
        path: &Path,
        text: Option<String>,
    ) {
        
        let reason = self.repository
            .entry(repo)
            .or_insert_with(Reason::new);

        reason.confidence +=1;
        reason.evidence.push(
            Evidence {
                file: path.to_path_buf(),
                text
            }
        )
    }
    
    /// This scans the current directory and its children
    pub fn detect(&mut self) -> Result<()> {
        // If called twice, it will clear out the old report and
        // scan fresh
        self.project.clear();
        self.repository.clear();
        
        let walker = WalkDir::new(Path::new("."))
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !should_exclude(e));

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

                                let explanation = match filename.to_str() {
                                    Some("Cargo.toml") => "Rust package manifest",
                                    Some("Cargo.lock") => "Rust dependency lockfile",
                                    _ => "Rust project file"
                                }.to_owned();
                                
                                match filename.to_str() {
                                    Some(fname) if matches!(fname, "Cargo.toml" | "Cargo.lock") => {
                                        self.insert_project(Project::Rust, path, Some(explanation));
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
                                                    let destination = path.with_file_name("Cargo.toml");
                                                    fs::rename(path, &destination)?;

                                                    self.insert_project(Project::Rust, &destination, Some(explanation));
                                                    
                                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                                    let destination = path.with_file_name("Cargo.lock");
                                                    fs::rename(path, &destination)?;

                                                    self.insert_project(Project::Rust, &destination, Some(explanation));
                                                    
                                                }
                                            },
                                            _ => {
                                                if filename.eq_ignore_ascii_case("cargo.toml") {
                                                    self.insert_project(Project::Rust, path, Some(explanation));
                                                } else if filename.eq_ignore_ascii_case("cargo.lock") {
                                                    self.insert_project(Project::Rust, path, Some(explanation));
                                                }
                                            },
                                        }

                                    }
                                }
                        } else if filename.eq_ignore_ascii_case("package.json") {

                            let explanation = String::from("JavaScript/TypeScript package manifest");
                            
                            match filename.to_str() {
                                Some("package.json") => {
                                    self.insert_project(Project::Javascript, path, Some(explanation));
                                },
                                _ => {
                                    eprintln!("\nWARNING: Non standard filename found {}!", path.display());
                                    eprintln!("JS/TS tools often expect exactly 'package.json' !\n");

                                    match take_input()?.trim() {
                                        "y" | "yes" => {
                                            let destination = path.with_file_name("package.json");
                                            fs::rename(path, &destination)?;
                                            self.insert_project(Project::Javascript, &destination, Some(explanation));
                                            
                                        },
                                        _ => {
                                            self.insert_project(Project::Javascript, &path, Some(explanation));
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
                            let explanation = String::from("git directory");
                                
                            match dir_name.to_str() {
                                Some(".git") => {
                                    self.insert_repository(Repository::Git, path, Some(explanation));
                                },
                                _ => {
                                    eprintln!("\nWARNING: Non standard directory found {}!", path.display());
                                    eprintln!("Other tools expect exactly '.git' !\n");

                                    match take_input()?.trim() {
                                        "y" | "yes" => {
                                            let destination = path.with_file_name(".git");
                                            fs::rename(path, &destination)?;
                                            self.insert_repository(Repository::Git, &destination, Some(explanation));
                                            
                                        },
                                        _ => {
                                            self.insert_repository(Repository::Git, path, Some(explanation));
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

fn format_reason(reason: &Reason, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    writeln!(f, "   Confidence: {}", reason.confidence)?;

    if !reason.evidence.is_empty(){
        writeln!(f, "   File: ")?;
        for evidence in &reason.evidence {

            if let Some(expl) = &evidence.text {
                writeln!(f,
                    "     - {}    > {expl}",
                    evidence.file.display()
                )?;
            } else {
                writeln!(f, "     - {}", evidence.file.display())?;
            }
        }
    }

    Ok(())
}

impl std::fmt::Display for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\n====Detection Report====")?;
        
        if self.repository.is_empty() && self.project.is_empty() {
            writeln!(f, "\nReport not ready yet!")?;
            
        } 

        if !self.repository.is_empty() {
            for (repository, reason) in &self.repository {
                writeln!(f, "\n------------------------")?;
                writeln!(f, "Repository: {repository}")?;
                writeln!(f, "Reason: ")?;

                format_reason(reason, f)?;

                writeln!(f, "------------------------\n")?;
            }
        }
        
        if !self.project.is_empty() {
            for (project, reason) in &self.project {
                writeln!(f, "\n------------------------")?;
                writeln!(f, "Project: {project}")?;
                writeln!(f, "Reason:")?;

                format_reason(reason, f)?;
                
                writeln!(f, "------------------------\n")?;
            }
        } 
        
        writeln!(f, "\n========================")
    }
}
