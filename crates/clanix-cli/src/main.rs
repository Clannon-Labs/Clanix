
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json;
// use std::process::Command;
use std::{fs, path::PathBuf};
use toml;

use clanix_core::ExecutionRequest;

const NAME: &str = "Clanix";

#[derive(Parser, Debug)]
#[command(name = "clanix")]
#[command(version)]
#[command(about = "Clanix is a fast developer execution tool built by Clannon Labs - Cybro")]
struct CliInput {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long)]
    verbose: bool,

    #[arg(short, long)]
    snapshot: Option<PathBuf>, // Use -s filename to snapshot output to a file

    #[arg(
        short = 'f',
        long = "format",
        requires = "snapshot",
        value_enum,
        default_value_t = OutputFormat::Json
    )]
    output_format: OutputFormat,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run { 
        program: String,
        args: Vec<String>
    },
    Test,

    Write {
        content: String,
        path: PathBuf
    },
}

#[derive(Debug, Clone, ValueEnum, Serialize, Deserialize)]
enum OutputFormat {
    Text,
    Json,
    Toml,
}

#[derive(Debug, Clone, Serialize)]
struct CommandResult {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn main() {
    let CliInput {
        command,
        verbose,
        snapshot,
        output_format,
    } = CliInput::parse();

    let mut result = CommandResult {
        success: false,
        code: None,
        stdout: "".to_string(),
        stderr: "".to_string(),
    };

    match command {
        Commands::Run { program, args } => {
            // let raw_result = Command::new(&program).args(&args).output().unwrap(); // Replace this later
            let request = ExecutionRequest {
                program: program.clone(),
                args: args.clone(),
            };
            let raw_result = request.execute().unwrap();
            // println!("Raw result: {:?}", raw_result);
            
            let args = args.join(" ");

            // We display stdout incase of success and stderr incase of failure
            if raw_result.status.success() {
                let output = String::from_utf8_lossy(&raw_result.stdout);

                if verbose {
                    let message = format!("{NAME}: Command {program} {args} was ran successfully!\n\n");
                    result.stdout = output.clone().into_owned();
                    println!("{message} {output}");
                } else {
                    let message = format!("{NAME}: {program} {args} was successful!\n\n");
                    result.stdout = output.clone().into_owned();
                    println!("{message} {output}");
                }
            } else {
                let output = String::from_utf8_lossy(&raw_result.stderr);

                if verbose {
                    let message = format!("{NAME}: Command {program} {args} failed!\n\n");
                    result.stderr = output.clone().into_owned();
                    eprintln!("{message} {output}");
                } else {
                    let message = format!("{NAME}: {program} {args} failed!\n\n");
                    result.stderr = output.clone().into_owned();
                    eprintln!("{message} {output}");
                }
            }

            result.success = raw_result.status.success();
            result.code = raw_result.status.code();
        }
        Commands::Test => {
            if verbose {
                let message = format!("{NAME} [SIMULATED]: Request to run tests was successful");
                println!("{}", &message);
                result.stdout = message;
            } else {
                println!("[SIMULATED]: Tests successful!");
                result.stdout = "[SIMULATED]: Tests successful!".to_string();
            }

            result.success = true;
            result.code = Some(101);
            result.stderr = "".to_string();
        }
        Commands::Write { content, path } => {
            fs::write(&path, &content).unwrap(); // Replace later

            if verbose {
                let message = format!(
                    "{NAME} [SIMULATED]: Request to write the content '{content}' to {path:?} was successful"
                );
                println!("{}", &message);
                result.stdout = message;
            } else {
                println!("[SIMULATED]: Wrote to {:?}", path);
                result.stdout = format!("Wrote to {path:?}");
            }

            result.success = true;
            result.code = Some(101);
            result.stderr = "".to_string();
        }
    }

    match snapshot {
        Some(path) => {
            match output_format {
                // No verbose check because inside the match command block,
                // each command already wrote their verbose output
                OutputFormat::Text => fs::write(&path, &result.stdout).unwrap(),
                OutputFormat::Json => {
                    let output = serde_json::to_string_pretty(&result).unwrap();
                    fs::write(&path, &output).unwrap()
                }
                OutputFormat::Toml => {
                    let output = toml::to_string_pretty(&result).unwrap();
                    fs::write(&path, &output).unwrap()
                }
            }
        }
        None => {}
    }
}
