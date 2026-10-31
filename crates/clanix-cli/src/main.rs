//! clanix-cli is the client that uses the abstraction/apis
//! provided by the clanix library to do the tasks the user wants.
//!
//! Clients just handles the user facing part and doesn't worry about how the actual
//! work is done behind the scenes by clanix.
//!
//! Clients can design the user facing api/commands however they want and clanix doesn't
//! care about any of that, all the client needs to do is use the structs/methods provided
//! by clanix, doesn't matter how client wants to use them to make the user facing cli
//! or whatever the client does

mod error;
mod handle;

use crate::error::Error;
use handle::*;

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use color_print::cformat;
use serde::{Deserialize, Serialize};
use humantime_serde::re::humantime;

use clanix::{Detector, Environment};

const NAME: &str = "Clanix";
const CYBRO_GITHUB: &str = "https://github.com/thecybro";
const REPO: &str = "https://github.com/Clannon-Labs/Clanix";
const DOCS: &str = "https://docs.rs/clanix";
const CRATE: &str = "https://crates.io/crates/clanix";

#[derive(Parser, Debug)]
#[command(name = "clanix")]
#[command(version)]
#[command(
about = cformat!("
    
<bold>{NAME}</> is a fast developer execution and observation tool;
built by creator of <bold>Clannon Labs</>:- <bold, underline>Cybro</>.
Official account: <blue, underline>{CYBRO_GITHUB}</> 
    
Visit <blue, underline>{REPO}</> to see the official repo
Visit <blue, underline>{DOCS}</> to read the documents
Visit <blue, underline>{CRATE}</> to know more about the crate/get it"
))]
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
        help = "To get a more detailed/verbose output from clanix"
    )]
    verbose: bool,

    
    #[arg(long, help="To get the output/stream of any process immediately")]
    stream: bool,
    
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
        help = "The format you want your snapshots to be"
    )]
    output_format: OutputFormat,

    /// To disable timeout for any one operation
    /// 
    /// This currently has a limitation, if your timeout is already disabled
    /// in the config, you have to set the persistent config manually to 
    /// make it timeout_disabled: false or timeout_enabled: true
    #[arg(
        long = "disable-timeout",
        visible_alias = "dt",
        help = "To disable the timeout you have saved; for one operation"
    )]
    disable_timeout: bool,

    /// The actual timeout you want to set
    #[arg(
        short,
        long,
        value_parser = parse_strict_timeout,
        default_value = "30s",

        help = "The actual timeout you want to set")]
    timeout: Option<Duration>,
}

/// The commands available to use
#[derive(Subcommand, Debug)]
enum Commands {
    /// A basic command you can use to run something inside clanix
    #[command(about = "To run something inside clanix")]
    Run {

        program: String,

        args: Vec<String>,
    },

    /// Just simulates test for now, we can put actual testing in future
    #[command(about = "To test/run the tests in the project")]
    Test,

    #[command(about = "To initialize the clanix project")]
    Init,

    /// This detects what type of project are you running clanix on
    ///
    /// Note that currently, it is not that useful as the detection mechanisms are very
    /// basic, but once those mechanisms are built properly, it can be quite useful
    /// for clanix to automate the tasks without requiring dog fooding from the user
    #[command(about = "To detect the type of the project")]
    What,

    /// To write some content to disk
    #[command(about = "To write some content to disk")]
    Write { content: String, path: PathBuf },

    /// All the configs are either read from the config file or
    /// set by core, cli doesn't do it itself.
    ///
    /// Cli can just change the config for any one operation only
    /// unless you explicitly call the 'default' and 'set' commands
    #[command(about = "To manage config. Run clanix config --help to get more info")]
    Config {
        #[command(subcommand)]
        configcommand: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Set/change individual config value.
    #[command(about = "Set/change individual config value.")]
    Set {
        /// Set the config back to the default (use carefully)
        #[arg(long, help = "Set the config back to the default (use carefully)")]
        default: bool,

        /// Change/set the config value stored in disable_timeout
        #[arg(
            long = "disable-timeout",
            visible_alias = "dt",
            help = "Change/set the config value stored in disable_timeout"
        )]
        disable_timeout: Option<bool>,

        /// Change/set the config value stored in timeout
        #[arg(
            short,
            long,
            value_parser = parse_strict_timeout,
            default_value = "30s",
            
            help = "Change/set the config value stored in timeout")]
        timeout: Option<Duration>,

        /// Change/set the config value stored in working_dir
        #[arg(
            long = "working-dir",
            visible_alias = "wd",
            help = "Change/set the config value stored in working_dir"
        )]
        working_dir: Option<PathBuf>,
    },

    /// Get the values you have in config.
    #[command(about = "Get the values you have in config.")]
    What {
        /// Get the config value stored in disable_timeout
        #[arg(
            long = "disable-timeout",
            visible_alias = "dt",
            help = "Get the config value stored in disable_timeout"
        )]
        disable_timeout: bool,

        /// Get the config value stored in timeout
        #[arg(short, long, help = "Get the config value stored in timeout")]
        timeout: bool,

        /// Get the config value stored in working_dir
        #[arg(
            long = "working-dir",
            visible_alias = "wd",
            help = "Get the config value stored in working_dir"
        )]
        working_dir: bool,
    },
}

/// The output formats available for the snapshots and whatever else
/// clanix saves on disk
#[derive(Debug, Clone, ValueEnum, Serialize, Deserialize)]
enum OutputFormat {
    Text,
    Json,
    Toml,
}

fn parse_strict_timeout(arg: &str) -> Result<Duration, String> {
    let trimmed = arg.trim();

    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        let secs = trimmed
            .parse::<u64>()
            .map_err(|e| format!("Invalid number: {}", e))?;

        return Ok(Duration::from_secs(secs));
    }

    humantime::parse_duration(trimmed).map_err(|_| {
        format!(
            "Invalid time format '{}'. Use suffixes h, m, s, ms, ns, us natively",
            arg
        )
    })
}


fn main() {
    if let Err(err) = clanix_cli() {
        let error_label = cformat!("<bold><red>✖ Clanix error</>");
        eprintln!("\n{error_label}\n  {err}\n");

        std::process::exit(1);
    }
}

fn clanix_cli() -> Result<(), Error> {
    let CliInput {
        command,
        verbose,
        stream,
        snapshot,
        output_format,
        timeout,
        disable_timeout,
        working_dir,
    } = CliInput::parse();

    // Create a new environment for execution
    let mut environment = Environment::new()?;

    match command {
        Commands::Run { program, args} => {
            let _ = run::handle_run(
                &mut environment,
                program,
                args,

                stream,
                
                verbose,
                disable_timeout,
                timeout,
                working_dir,
            )?;

            // println!("{environment}");
        }
        Commands::Test => {
            let _ = test::handle_test(&mut environment, verbose);
        }
        Commands::Init => {
            let notice = cformat!("<bold><yellow>Coming soon</>");
            println!("{notice}  Project initialization is not available yet.");
        }
        Commands::What => {
            // Give an instance of the detector
            let mut detector = Detector::new();
            // Detect the project
            detector.detect()?;

            let heading = cformat!("<bold><cyan>Project detection</>");
            // See the projects and their reasons on why they were detected.
            // 
            // Note that this currently shows a full verbose output in the terminal
            let reason = detector.why();
            println!("{heading}\n{reason}");
        }
        Commands::Write { content, path } => {
            let _ = write::handle_write(&mut environment, verbose, path, content);
        }
        Commands::Config { configcommand } => {
            let _ = config::handle_config(&mut environment, configcommand, verbose);
        }
    }

    match snapshot {
        Some(path) => {
            let _ = snapshot::handle_snapshot(&mut environment, path, output_format);
        }
        None => {}
    }

    Ok(())
}
