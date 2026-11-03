use std::fs;
use std::path::{Path, PathBuf};

use clanix::Environment;
use color_print::cformat;

use crate::OutputFormat;
use crate::error::Result;

pub fn handle_snapshot(
    environment: &mut Environment,
    path: PathBuf,
    output_format: OutputFormat,
) -> Result<()> {
    if let Some(result) = environment.last_result() {
        let final_path = result.working_dir().join(path);

        write(output_format, &final_path, &environment)?;

        let label = cformat!("<bold><green>✔Snapshot</>");
        println!("{label}\n Saved text snapshot to {}", final_path.display());
    }

    Ok(())
}

fn write(output_format: OutputFormat, path: &Path, environment: &Environment) -> Result<()> {
    match output_format {
        // No verbose check because inside the match command block,
        // each command already wrote their verbose output
        OutputFormat::Text => {
            fs::write(&path, &environment.to_string())?;

            Ok(())
        }
        OutputFormat::Json => {
            fs::write(&path, &environment.to_json()?)?;

            Ok(())
        }
        OutputFormat::Toml => {
            fs::write(&path, &environment.to_toml()?)?;

            Ok(())
        }
    }
}
