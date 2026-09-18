use clap::{Parser, Subcommand};
use std::{
    path::PathBuf, 
    fs,
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

    #[arg(short, long)]
    output: Option<PathBuf>,
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

fn main() {
    let CliInput {command, verbose, output} = CliInput::parse();

    match command {
        Commands::Run {program, args} => {
            let content = format!("\nProgram: {program}\nArgs: {args:?}\nVerbose: {verbose}");
            match output {
                Some(output) => {
                    fs::write(&output, &content).unwrap()
                },
                None => {
                    println!("{}", content)
                }
            }
        },
        Commands::Test => {
            let content = format!("\nRan tests\nVerbose: {verbose}");
            match output {
                Some(output) => {
                    fs::write(&output, &content).unwrap()
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
            match output {
                Some(output) => {
                    // Write the function result to output path
                    fs::write(&output, &result).unwrap();
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
