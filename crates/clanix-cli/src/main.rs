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

mod handle;
mod error;

use handle::*;
use crate::error::Error;

use std::path::PathBuf;

use clap::{
    Parser,
    Subcommand,
    ValueEnum,
};
use serde::{Deserialize, Serialize};

use clanix::Environment;

const NAME: &str = "Clanix";

#[derive(Parser, Debug)]
#[command(name = "clanix")]
#[command(version)]
#[command(
about = "{NAME} is a fast developer execution tool built by Clannon Labs - Cybro
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
        help = "The actual timeout you want to set(in seconds)"
    )]
    timeout: Option<u64>,
}

/// The commands available to use
#[derive(Subcommand, Debug)]
enum Commands {
    /// A basic command you can use to run something inside clanix
    #[command(about = "A basic command you can use to run something inside clanix")]
    Run { 
        program: String,
        args: Vec<String>,
    },

    /// Just simulates test for now, we can put actual testing in future
    #[command(about = "Just simulates test for now, we can put actual testing in future")]
    Test,

    /// To write some content to disk
    #[command(about = "To write some content to disk")]
    Write {
        content: String,
        path: PathBuf
    },

    /// All the configs are either read from the config file or 
    /// set by core, cli doesn't do it itself.
    /// 
    /// Cli can just change the config for any one operation only
    /// unless you explicitly call the 'default' and 'set' commands
    #[command(about = "Command to manage config. Run clanix config --help to get more info")]
    Config {
        #[command(subcommand)]
        configcommand: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    #[command(about = "Set the config back to the default (use carefully")]
    Default,

    #[command(about = "Initialize the clanix project")]
    Init,

    #[command(about = "Set/change individual config value.")]
    Set {
        #[arg(
            long = "disable-timeout",
            visible_alias = "dt",
            help = "Change/set the config value stored in disable_timeout"
        )]
        disable_timeout: Option<bool>,

        #[arg(
            short, 
            long,
            help = "Change/set the config value stored in timeout_s"
        )]
        timeout_s: Option<u64>,

        #[arg(
            long = "working-dir",
            visible_alias = "wd",
            help = "Change/set the config value stored in working_dir"
        )]
        working_dir: Option<PathBuf>,
    },

    #[command(about = "Get the values you have in config.")]
    What {
        #[arg(
            long = "disable-timeout",
            visible_alias = "dt",
            help = "Get the config value stored in disable_timeout"
        )]
        disable_timeout: bool,

        #[arg(
            short, 
            long,
            help = "Get the config value stored in timeout_s"
        )]
        timeout_s: bool,

        #[arg(
            long = "working-dir",
            visible_alias = "wd",
            help = "Get the config value stored in working_dir"
        )]
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


fn main() -> Result<(), Error> {
    let CliInput {
        command,
        verbose,
        snapshot,
        output_format,
        timeout,
        disable_timeout,
        working_dir
    } = CliInput::parse();

    // Create a new environment for execution
    let mut environment = Environment::new()?;
    
    // Get the result type
    let mut result = environment.new_result()?;

    match command {
        Commands::Run { program, args } => {
            // Run in that environment with these arguments
            let request = environment.new_request(&program)?
                    .args(&args)
                    .set_disable_timeout(disable_timeout)?
                    .set_timeout_s(timeout)?
                    .set_working_dir(working_dir)?;

            // IMPORTANT: Execute the request in that environment because
            // running the request outside of the environment will make the
            // request and error drift apart from the rest of the environment..
            // 
            environment = environment.execute_request(request)?;
            
            // Get the result form the environment and replace the old default result
            result = environment.result()?.to_owned();
            
            let _ = run::handle_run(&mut result, program, args, verbose);

            println!("{environment}");
        }
        Commands::Test => {
            let _ = test::handle_test(&mut result, verbose);
        }
        Commands::Write { content, path } => {
            let _ = write::handle_write(&mut result, verbose, path, content);
        },
        Commands::Config {configcommand} => {
            let _ = config::handle_config(&mut result, configcommand, verbose);
        }
    }

    match snapshot {
        Some(path) => {
            let _ = snapshot::handle_snapshot(&mut result, path, output_format);
        }
        None => {}
    }

    Ok(())
}
