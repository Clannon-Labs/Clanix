use clap::{Parser, Subcommand};
use std::{
    path::PathBuf, 
    fs::{
        self,
        File,
    },
};

#[derive(Parser, Debug)]
#[command(name = "clanix")]
#[command(version)]
#[command(about = "A fast developer execution tool")]
struct CliInput {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run {
        program: String,

        args: Vec<String>,
    },
    Test,

    Write {
        path: PathBuf,

        content: String,
    }
}

fn main() {
    let CliInput {command, verbose} = CliInput::parse();

    match command {
        Commands::Run {program, args} => {
            println!("\nProgram: {}\nArgs: {:?}\nVerbose: {}", program, args, verbose)
        },
        Commands::Test => {
            println!("\nRunning tests\nVerbose: {}", verbose)
        },
        Commands::Write {path, content} => {
            File::create(&path).unwrap();
            fs::write(&path, &content).unwrap();
            println!("\nWrote {} to {:?}\nVerbose: {}", content, path, verbose)
        }
    }

}
