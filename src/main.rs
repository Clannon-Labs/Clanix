use clap::{Parser, Subcommand, ValueEnum};
use std::{
    path::PathBuf, 
    fs,
};
use std::process::{Command, Output as ProcessOutput};
use serde::{Serialize, Deserialize};
use serde_json;
use toml;

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
    global_format: OutputFormat,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run {
        program: String,

        args: Vec<String>,

        #[arg(short, long)]
        path: Option<PathBuf>,

        #[arg(
            short = 'f',
            long = "format",
            requires = "path",
            value_enum,
            default_value_t = OutputFormat::Json
        )]
        run_format: OutputFormat,
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
    let CliInput {command, verbose, save, global_format} = CliInput::parse();

    match command {
        Commands::Run {program, args, path, run_format} => {
            let raw_result = Command::new(&program).args(&args).output().unwrap();
            
            let content = content_from_result(&raw_result, &global_format);
            
            let args = args.join(" ");
            let cmd_status_msg = format!("Command: '{program} {args}' ran successfully!");
            
            match save {
                Some(save_path) => {
                    println!("Global save flag found! Saving to its path instead.");
                    
                    fs::write(&save_path, &content).unwrap();
                    println!("Output has been saved to file: {:?}", save_path)
                },
                None => {
                    let content = content_from_result(&raw_result, &run_format);
                    match path {
                        Some(path) => {
                            fs::write(&path, &content).unwrap();
                            println!("Output has been saved to file: {:?}", path)
                        },
                        None => {}
                    }
                    
                    println!("{}", cmd_status_msg);
                }
            }
        },
        Commands::Test => {
            // let result = 
            let content = format!("\nRan tests(fake) \nVerbose: {verbose}");
            match save {
                Some(save_path) => {
                    fs::write(&save_path, &content).unwrap()
                },
                None => {
                    println!("{}", content)
                }
            }
        },
        Commands::Write {path, content} => {
            /*
             * Write content to the given file inside Write
             * and result to the given output
             */

            
            let result = format!("\nWrote {content} to {path:?}\nVerbose: {verbose}");
            match save {
                Some(save_path) => {
                    // Write the function result to save_path path
                    fs::write(&save_path, &result).unwrap();
                    // Write the content to the given file
                    fs::write(&path, &content).unwrap()
                },
                None => {
                    println!("{}", result)
                }
            }
        }
    }
}


fn content_from_result(raw_result: &ProcessOutput, format: &OutputFormat) -> String {
    let result = CommandResult {
        success: raw_result.status.success(),
        code: raw_result.status.code(),
        stdout: String::from_utf8_lossy(&raw_result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&raw_result.stderr).into_owned(),
    };
    // global_format always has a value
    match format {
        OutputFormat::Text => {
            if result.success {
                // println!("Success: {}", result.success);
                let output = String::from_utf8_lossy(&raw_result.stdout).into_owned();
                println!("{}", output);
                
                return output
            } else {
                // println!("Success: {}", result.success);
                return String::from_utf8_lossy(&raw_result.stderr).into_owned()
            }
        },
        OutputFormat::Json => {
            return serde_json::to_string_pretty(&result).unwrap()
        },
        OutputFormat::Toml => {
            return toml::to_string_pretty(&result).unwrap()
        }
    }
}
