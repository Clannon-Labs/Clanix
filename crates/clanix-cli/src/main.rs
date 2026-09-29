//! clanix-cli is the client that uses the abstraction
//! provided by the clanix library to do the tasks the user wants.
//! 
//! Clients just handles the user facing part and doesn't worry about how the actual
//! work is done behind the scenes by clanix.
//! 
//! Clients can design the user facing api/commands however they want and clanix doesn't
//! care about any of that, all the client needs to do is use the structs/methods provided
//! by clanix, doesn't matter how client wants to use them to make the user facing cli
//! or whatever the client does

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fs;
use serde_json;
use toml;

use clanix::{
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
    /// The commands the user actually types
    /// For example: run, write, config etc
    #[command(subcommand)]
    command: Commands,

    /// These arguments only change the in-memory values/policies,
    /// not the persistent configs,
    /// To change the configs, you have to use `clanix config`

    /// The directory the operation actually happens
    #[arg(
        long = "working-dir",
        visible_alias = "wd",
        help = "The directory for this one operation"
    )] 
    working_dir: Option<PathBuf>,

    /// Option to enable/to disable verbosity.
    /// Having this enabled means clanix will give you more
    /// detailed/verbose outputs everywhere
    #[arg(
        short,
        long,
        help = "Use this flag to get a more detailed/verbose output from clanix"
    )]
    verbose: bool,

    /// Whether or not user wants to save/snapshot the output to a file
    #[arg(
        short,
        long,
        help = "The directory to save the snapshot to (if you use this flag)"
    )]
    snapshot: Option<PathBuf>, // Use -s filename to snapshot output to a file

    /// What format does the user want to save their snapshot..
    /// Default is Json as you can see here
    #[arg(
        short = 'f',
        long = "format",
        requires = "snapshot",
        value_enum,
        default_value_t = OutputFormat::Json,
        help = "What format do you want your snapshots to be"
    )]
    output_format: OutputFormat,

    /// To disable timeout for any one operation
    /// You can use --dt true to disable whatever timeout you had set
    /// And you can use --dt false to enable (or not disable) the timeout
    /// you have
    #[arg(
        long = "disable-timeout",
        visible_alias = "dt",
        help = "Use this flag to disable the timeout you have saved; for one operation"
    )]
    disable_timeout: bool,

    /// The actual timeout you want to set
    #[arg(
        short,
        long,
        help = "The actual timeout you want to set"
    )]
    timeout: Option<u64>,
}

/// The commands available to use
#[derive(Subcommand, Debug)]
enum Commands {
    /// A basic command you can use to run something inside clanix
    Run { 
        program: String,
        args: Vec<String>,
    },

    /// Just simulates test for now, we can put actual testing in future
    Test,

    /// To write some content to disk
    Write {
        content: String,
        path: PathBuf
    },

    /// All the configs are either read from the config file or 
    /// set by core, cli doesn't do it itself.
    /// 
    /// Cli can just change the config for any one operation only
    Config {
        #[command(subcommand)]
        configcommand: ConfigCommands,
    }
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    #[command(about = "Set the config back to the default (use carefully")]
    Default,

    #[command(about = "Initialize the clanix project")]
    Init,

    #[command(about = "Set/change the config values")]
    Set {
        #[arg(long = "disable-timeout", visible_alias = "dt")]
        disable_timeout: Option<bool>,

        #[arg(short, long)]
        timeout_ms: Option<u64>,

        #[arg(long = "working-dir", visible_alias = "wd" )]
        working_dir: Option<PathBuf>,
    },

    #[command(about = "Get the values you have in config")]
    What {
        #[arg(long = "disable-timeout", visible_alias = "dt")]
        disable_timeout: bool,

        #[arg(short, long)]
        timeout_ms: bool,

        #[arg(long = "working-dir", visible_alias = "wd" )]
        working_dir: bool,
    }
}

/// The output formats available for the snapshots and whatever else
/// clanix saves on disk
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
                    let message = format!("{NAME}: Command {program} {args} was ran successfully!\n{final_output}\n");
                    println!("{message}");
                    result.set_stdout(message);
                } else {
                    let message = format!("{NAME}: {program} {args} was successful!\n{final_output}\n");
                    println!("{message}");
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
                    let message = format!("{NAME}: Command {program} {args} failed!\n{output}\n");
                    eprintln!("{message}");
                    result.set_stderr(final_output)
                } else {
                    let message = format!("{NAME}: {program} {args} failed!\n{output}\n");
                    eprintln!("{message}");
                    result.set_stderr(final_output)
                }
            }

            // result.success = result.success();
            // result.code = result.status();
        }
        Commands::Test => {
            if verbose {
                let message = format!("{NAME} [SIMULATED]: Request to run tests was successful\n");
                println!("{message}");
                result.set_stdout(message);
            } else {
                let message = "[SIMULATED]: Tests successful!".to_string();
                println!("{message}");
                result.set_stdout(message);
            }

            result.set_success(true);
            result.set_status(Some(101));
            result.set_stderr("".to_string());
        }
        Commands::Write { content, path } => {
            fs::write(&path, &content).unwrap(); // Replace later

            if verbose {
                let message = format!(
                    "{NAME} [SIMULATED]: Request to write the content '{content}' to {path:?} was successful\n"
                );
                println!("{message}");
                result.set_stdout(message);
            } else {
                let message = "[SIMULATED]: Wrote to {path:?}".to_string();
                println!("{message}");
                result.set_stdout(message);
            }

            result.set_success(true);
            result.set_status(Some(101));
            result.set_stderr("".to_string());
        },
        Commands::Config {configcommand} => {
            /*
             * The core could technically be able to set the config to default if
             * we called .set_default() at the end of the chain but it would be 
             * a waste of resources, so we check the condition and set to default if
             * user wants so
             */
            
            // println!("{disable_timeout:?}, {timeout_ms:?}, {working_dir:?}");
            match configcommand {
                ConfigCommands::Default => {
                    let config = Config::set_default();
    
                    // println!("{config:?}");
                    if verbose {
                        let message = format!("{NAME}: Successfully set the following config:\n{config:?}\n");
                        println!("{message}");
                        result.set_stdout(message);
                    } else {
                        let message = format!("{NAME}: Successfully set the config.\n");
                        println!("{message}");
                        result.set_stdout(message);
                    }
                },
                ConfigCommands::Init => {
                    println!("Init is not developed yet!")
                },
                ConfigCommands::Set {disable_timeout, timeout_ms, working_dir} => {
                    let config = Config::new()
                        .set_disable_timeout(disable_timeout)
                        .set_timeout_ms(timeout_ms)
                        .set_working_dir(working_dir);
                    
                    if verbose {
                        let message = format!("{NAME}: Successfully set the following config:\n{config:?}\n");
                        println!("{message}");
                        result.set_stdout(message);
                    } else {
                        let message = format!("{NAME}: Successfully set the config.\n");
                        println!("{message}");
                        result.set_stdout(message);
                    }
                },
                ConfigCommands::What{disable_timeout, timeout_ms, working_dir} => {
                    let config = Config::read();
                    if disable_timeout {
                        println!("{}", config.disable_timeout())
                    } else if timeout_ms {
                        println!("{:?}", config.timeout_ms())
                    } else if working_dir {
                        println!("{:?}", config.working_dir())
                    } else {
                        println!("{:?}", config)
                    }
                }
            }

            result.set_success(true);
            result.set_status(Some(101));
            result.set_stderr("".to_string());
        }
    }

    match snapshot {
        Some(path) => {
            match output_format {
                // No verbose check because inside the match command block,
                // each command already wrote their verbose output
                OutputFormat::Text => {
                    let final_path = result.working_dir().join(path);
                    fs::write(&final_path, &result.stdout()).unwrap();
                    println!("Wrote stdout to {final_path:?}")
                },
                OutputFormat::Json => {
                    let final_path = result.working_dir().join(path);
                    let output = serde_json::to_string_pretty(&result).unwrap();
                    fs::write(&final_path, &output).unwrap();
                    println!("Wrote stdout to {final_path:?}")
                }
                OutputFormat::Toml => {
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
