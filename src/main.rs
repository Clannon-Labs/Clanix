use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clanix")]
struct CliInput {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        project: String,

        #[arg(long, short, default_value_t=false)]
        snapshot: bool,
    },
}

fn main() {
    let input = CliInput::parse();

    match input.command {
        Commands::Run {project, snapshot} => {
            println!("\nRunning project: {}\nsnapshot: {:?}\n", project, snapshot);
        }
    }
}
