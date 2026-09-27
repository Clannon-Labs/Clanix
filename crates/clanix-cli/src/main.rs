
use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fs;
use serde_json;
use toml;

use clanix_core::{
    Config,
    PolicyTrait,
    ExecutionResult,
    ExecutionRequest,
};

const NAME: &str = "Clanix";

#[derive(Parser, Debug)]
#[command(name = "clanix")]
#[command(version)]
#[command(
about = "Clanix is a fast developer execution tool built by Clannon Labs - Cybro
Visit https://github.com/Clannon-Labs/Clanix to read the documents"
)]
struct CliInput {
    #[command(subcommand)]
    command: Commands,

    /*
     * These arguments only change the in-memory policies,
     * not the persistent configs,
     * To change the configs, you have to use `clanix config`
     */

    #[arg(long = "working-dir", visible_alias = "wd" )] 
    working_dir: Option<PathBuf>,
    
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

    #[arg(long = "disable-timeout", visible_alias = "dt")]
    disable_timeout: bool,
    
    #[arg(short, long)]
    timeout: Option<u64>,
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
        path: PathBuf
    },

    // all the configs are either read from the config file or 
    // set by core, cli doesn't do it itself
    Config {
        #[arg(short, long = "default", visible_alias = "init" )]
        default: bool,
        
        #[arg(long = "disable-timeout", visible_alias = "dt")]
        disable_timeout: bool,

        #[arg(short, long)]
        timeout_ms: Option<u64>,

        #[arg(long = "working-dir", visible_alias = "wd" )]
        working_dir: Option<PathBuf>,
    }
}

#[derive(Debug, Clone, ValueEnum, Serialize, Deserialize)]
enum OutputFormat {
    Text,
    Json,
    Toml,
}

fn main() {
    let CliInput {
        command,
        verbose,
        snapshot,
        output_format,
        timeout,
        disable_timeout,
        working_dir
    } = CliInput::parse();

    let mut result = ExecutionResult::defaults(&working_dir);

    match command {
        Commands::Run { program, args } => {
            result = ExecutionRequest::new(&program)
                .args(&args)
                .set_disable_timeout(disable_timeout)
                .set_timeout_ms(timeout)
                .set_working_dir(working_dir)
                .execute()
                .unwrap();
            
            // println!("Execution {} was just ran", &result.id());
            
            let args = args.join(" ");

            let stdout_output = &result.stdout_string();
            let stderr_output = &result.stderr_string();
            
            // We display stdout incase of success and stderr incase of failure
            // But some commands might run successfully but put the content in stderr
            // so we check if there's anything on stderr incase we find stdout empty 
            // even incase of success
            if result.success() {
                let output = if !stdout_output.trim().is_empty() {
                    stdout_output
                } else {
                    stderr_output
                };
                
                let final_output = output.clone().into_owned();
                if verbose {
                    let message = format!("{NAME}: Command {program} {args} was ran successfully!\n\n{final_output}");
                    println!("Following stdout was set: \n\n{message}");
                    result.set_stdout(message);
                } else {
                    let message = format!("{NAME}: {program} {args} was successful!\n\n{final_output}");
                    println!("Following stdout was set: \n\n{message}");
                    result.set_stdout(message)
                }
            } else {
                let output = if !stderr_output.trim().is_empty() {
                    stderr_output
                } else {
                    stdout_output
                };

                let final_output = output.clone().into_owned();
                if verbose {
                    let message = format!("{NAME}: Command {program} {args} failed!\n\n{output}");
                    eprintln!("Following stdout was set: \n\n{message}");
                    result.set_stderr(final_output)
                } else {
                    let message = format!("{NAME}: {program} {args} failed!\n\n{output}");
                    eprintln!("Following stdout was set: \n\n{message}");
                    result.set_stderr(final_output)
                }
            }

            // result.success = result.success();
            // result.code = result.status();
        }
        Commands::Test => {
            if verbose {
                let message = format!("{NAME} [SIMULATED]: Request to run tests was successful");
                println!("{}", &message);
                result.set_stdout(message);
            } else {
                println!("[SIMULATED]: Tests successful!");
                result.set_stdout("[SIMULATED]: Tests successful!".to_string());
            }

            result.set_success(true);
            result.set_status(Some(101));
            result.set_stderr("".to_string());
        }
        Commands::Write { content, path } => {
            fs::write(&path, &content).unwrap(); // Replace later

            if verbose {
                let message = format!(
                    "{NAME} [SIMULATED]: Request to write the content '{content}' to {path:?} was successful"
                );
                println!("{}", &message);
                result.set_stdout(message);
            } else {
                let message = "[SIMULATED]: Wrote to {path:?}".to_string();
                println!("Following stdout was set: \n\n{message}");
                result.set_stdout(message);
            }

            result.set_success(true);
            result.set_status(Some(101));
            result.set_stderr("".to_string());
        },
        Commands::Config { disable_timeout, timeout_ms, working_dir, default } => {
            
            /*
             * The core could technically be able to set the config to default if
             * we called .set_default() at the end of the chain but it would be 
             * a waste of resources, so we check the condition and set to default if
             * user wants so
             */
            
            // println!("{disable_timeout:?}, {timeout_ms:?}, {working_dir:?}");
            if default {
                let config = Config::set_default(default);

                // println!("{config:?}");
                if verbose {
                    let message = format!("{NAME}: Successfully set the following config: {config:?}");
                    println!("Following stdout was set: \n\n{message}");
                    result.set_stdout(message);
                } else {
                    let message = format!("{NAME}: Successfully set the config.");
                    println!("Following stdout was set: \n\n{message}");
                    result.set_stdout(message);
                }
            } else {
                let config = Config::new()
                    .set_disable_timeout(disable_timeout)
                    .set_timeout_ms(timeout_ms)
                    .set_working_dir(working_dir);
                // println!("{config:?}");
                if verbose {
                    let message = format!("{NAME}: Successfully set the following config: {config:?}");
                    println!("Following stdout was set: \n\n{message}");
                    result.set_stdout(message);
                } else {
                    let message = format!("{NAME}: Successfully set the config.");
                    println!("Following stdout was set: \n\n{message}");
                    result.set_stdout(message);
                }
            }

            result.set_success(true);
            result.set_status(Some(101));
            result.set_stderr("".to_string());
        }
    }

    match snapshot {
        Some(path) => {
            println!("Snapshot requested");
            match output_format {
                // No verbose check because inside the match command block,
                // each command already wrote their verbose output
                OutputFormat::Text => {
                    println!("Text format requested");
                    let final_path = result.working_dir().join(path);
                    fs::write(&final_path, &result.stdout()).unwrap();
                    println!("Wrote stdout to {final_path:?}")
                },
                OutputFormat::Json => {
                    println!("Json format requested");
                    let final_path = result.working_dir().join(path);
                    let output = serde_json::to_string_pretty(&result).unwrap();
                    fs::write(&final_path, &output).unwrap();
                    println!("Wrote stdout to {final_path:?}")
                }
                OutputFormat::Toml => {
                    println!("Toml format requested");
                    let final_path = result.working_dir().join(path);
                    let output = toml::to_string_pretty(&result).unwrap();
                    fs::write(&final_path, &output).unwrap();
                    println!("Wrote stdout to {final_path:?}")
                }
            }
        }
        None => {}
    }
}
