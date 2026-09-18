use clap::{Parser, Subcommand};
use std::{
    path::PathBuf, 
    fs,
};
use std::process::Command;

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
    save: Option<PathBuf>, // Use -s filename to save output to a file
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run {
        program: String,

        args: Vec<String>,

        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    Test,

    Write {
        content: String,

        path: PathBuf,
    }
}

fn main() {
    let CliInput {command, verbose, save} = CliInput::parse();

    match command {
        Commands::Run {program, args, path} => {
            let result = Command::new(&program).args(&args).output().unwrap();
            let content = String::from_utf8_lossy(&result.stdout);
            let args = args.join(" ");
            println!("Command: '{} {}' ran successfully!", program, args);
            
            match save {
                Some(save_path) => {
                    println!("Global save flag found! Saving to its path instead.");
                    if content.trim().is_empty() {
                        println!("Trimmed content is empty");
                        let content = format!("Command: '{program} {args}' ran successfully!");
                        fs::write(&save_path, &content).unwrap()
                    } else {
                        println!("Content wasn't empty");
                        fs::write(&save_path, &*content).unwrap();
                        println!("Output has been saved to file: {:?}", save_path)
                    }
                },
                None => {
                    match path {
                        Some(path) => {
                            if content.trim().is_empty() {
                                println!("Trimmed content was empty!");
                                let content = format!("Command: '{program} {args}' ran successfully!");
                                fs::write(&path, &content).unwrap();
                                println!("Output has been saved to file: {:?}", path)
                            } else {
                                fs::write(&path, &*content).unwrap();
                                println!("Wrote '{}' to '{:?}'", content, path);
                            }
                        },
                        None => {}
                    }
                    println!("{}", content)
                }
            }
        },
        Commands::Test => {
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
