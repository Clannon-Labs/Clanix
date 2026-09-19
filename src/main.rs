use clap::{Parser, Subcommand, ValueEnum};
use std::{
    path::PathBuf, 
    fs,
};
use std::process::Command;
use serde::{Serialize, Deserialize};
use serde_json;
use toml;

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
    save: Option<PathBuf>, // Use -s filename to save output to a file

    #[arg(
        short = 'f',
        long = "format",
        requires = "save",
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
    },
    Test,

    Write {
        content: String,

        path: PathBuf,
    }
}

#[derive(Debug, Clone, ValueEnum, Serialize, Deserialize)]
enum OutputFormat {
    Text,
    Json,
    Toml
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
        save,
        output_format,
    } = CliInput::parse();

    let mut result = CommandResult {
        success: false,
        code: None,
        stdout: "".to_string(),
        stderr: "".to_string(),
    };
    
    match command {
        Commands::Run {program, args} => {
            let raw_result = Command::new(&program)
                .args(&args)
                .output()
                .unwrap(); // Replace this later

            let args = args.join(" ");
            
            // We display stdout incase of success and stderr incase of failure
            if raw_result.status.success() {
                let output = String::from_utf8_lossy(&raw_result.stdout);
                
                if verbose {
                    let message = format!("{NAME}: Command {program} {args:?} was ran successfully!\n");
                    result.stdout = format!("{message}: {output}");
                    println!("{}", output);
                } else {
                    result.stdout = output.clone().into_owned();
                    println!("{}", &output);
                }
            } else {
                let output = String::from_utf8_lossy(&raw_result.stderr).into_owned();
                
                if verbose {
                    let message = format!("{NAME}: Command {program} {args:?} failed!\n");
                    result.stderr = format!("{message}: {output}");
                    println!("{:?}", &raw_result.stderr);
                } else {
                    result.stderr = output;
                    println!("{:?}", &raw_result.stderr);
                }
            }

            result.success = raw_result.status.success();
            result.code = raw_result.status.code();
        },
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
        },
        Commands::Write {content, path} => {
            fs::write(&path, &content).unwrap(); // Replace later

            if verbose {
                let message = format!("{NAME} [SIMULATED]: Request to write the content '{content}' to {path:?} was successful");
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

    match save {
        Some(path) => {
            match output_format {
                // No verbose check because inside the match command block,
                // each command already wrote their verbose output
                OutputFormat::Text => {
                    fs::write(&path, &result.stdout).unwrap()
                },
                OutputFormat::Json => {
                    let output = serde_json::to_string_pretty(&result).unwrap();
                    fs::write(&path, &output).unwrap()
                },
                OutputFormat::Toml => {
                    let output = toml::to_string_pretty(&result).unwrap();
                    fs::write(&path, &output).unwrap()
                }
            }
        },
        None => {}
    }
}