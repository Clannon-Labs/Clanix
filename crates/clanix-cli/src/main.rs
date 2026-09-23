
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json;
use std::{fs, path::PathBuf};
use toml;

use clanix_core::{
    ExecutionRequest,
};

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
        args: Vec<String>,

        // This is the default set by the cli and the core
        // has its own default value incase cli doesn't give it any
        #[arg(short, long, default_value_t = 100)]
        timeout: u64,
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

// This is just a struct for testing out dummy commands
// like test and write and will be deleted/replaced later
#[derive(Debug, Clone, Serialize)]
struct TestResult {
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

    // Dummy result
    let mut test_result = TestResult {
        success: false,
        code: None,
        stdout: "".to_string(),
        stderr: "".to_string(),
    };

    match command {
        Commands::Run { program, args, timeout } => {
            let result = ExecutionRequest::new(&program)
                .args(&args)
                .timeout_ms(timeout)
                .execute().unwrap();
            
            // println!("Execution {} was just ran", &result.id());
            
            let args = args.join(" ");

            let stdout_output = &result.stdout_string();
            let stderr_output = &result.stderr_string();
            
            // We display stdout incase of success and stderr incase of failure
            if result.success() {
                let output = if !stdout_output.trim().is_empty() {
                    stdout_output
                } else {
                    stderr_output
                };
                
                if verbose {
                    let message = format!("{NAME}: Command {program} {args} was ran successfully!\n\n");
                    // result.set_stdout(output.into_owned());
                    println!("{} {}", message, output);
                } else {
                    let message = format!("{NAME}: {program} {args} was successful!\n\n");
                    // result.set_stdout(output.into_owned()); 
                    println!("{} {}", message, output);
                }
            } else {
                let output = if !stderr_output.trim().is_empty() {
                    stderr_output
                } else {
                    stdout_output
                };
                
                if verbose {
                    let message = format!("{NAME}: Command {program} {args} failed!\n\n");
                    // result.set_stderr(output.into_owned());
                    eprintln!("{} {:?}", message, output);
                } else {
                    let message = format!("{NAME}: {program} {args} failed!\n\n");
                    // result.set_stderr(output.into_owned());
                    eprintln!("{} {:?}", message, output);
                }
            }

            // result.success = result.success();
            // result.code = result.status();
        }
        Commands::Test => {
            if verbose {
                let message = format!("{NAME} [SIMULATED]: Request to run tests was successful");
                println!("{}", &message);
                test_result.stdout = message;
            } else {
                println!("[SIMULATED]: Tests successful!");
                test_result.stdout = "[SIMULATED]: Tests successful!".to_string();
            }

            test_result.success = true;
            test_result.code = Some(101);
            test_result.stderr = "".to_string();
        }
        Commands::Write { content, path } => {
            fs::write(&path, &content).unwrap(); // Replace later

            if verbose {
                let message = format!(
                    "{NAME} [SIMULATED]: Request to write the content '{content}' to {path:?} was successful"
                );
                println!("{}", &message);
                test_result.stdout = message;
            } else {
                println!("[SIMULATED]: Wrote to {:?}", path);
                test_result.stdout = format!("Wrote to {path:?}");
            }

            test_result.success = true;
            test_result.code = Some(101);
            test_result.stderr = "".to_string();
        }
    }

    match snapshot {
        Some(path) => {
            match output_format {
                // No verbose check because inside the match command block,
                // each command already wrote their verbose output
                OutputFormat::Text => fs::write(&path, &test_result.stdout).unwrap(),
                OutputFormat::Json => {
                    let output = serde_json::to_string_pretty(&test_result).unwrap();
                    fs::write(&path, &output).unwrap()
                }
                OutputFormat::Toml => {
                    let output = toml::to_string_pretty(&test_result).unwrap();
                    fs::write(&path, &output).unwrap()
                }
            }
        }
        None => {}
    }
}
