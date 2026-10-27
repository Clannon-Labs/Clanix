use std::fs;
use std::path::PathBuf;

use clanix::Environment;
use color_print::cformat;

use crate::OutputFormat;
use crate::error::Result;

pub fn handle_snapshot(
    environment: &mut Environment,
    path: PathBuf,
    output_format: OutputFormat,
) -> Result<()> {
    match output_format {
        // No verbose check because inside the match command block,
        // each command already wrote their verbose output
        OutputFormat::Text => {
            let final_path = environment.result()?.working_dir().join(path);

            fs::write(&final_path, &environment.to_string())?;
            let label = cformat!("<bold><green>✔Snapshot</>");
            println!("{label}\n Saved text snapshot to {}", final_path.display());

            Ok(())
        }
        OutputFormat::Json => {
            let final_path = environment.result()?.working_dir().join(path);

            fs::write(&final_path, &environment.to_json()?)?;
            let label = cformat!("<bold><green> Snapshot</>");
            println!("{label}\n Saved JSON snapshot to {}", final_path.display());

            Ok(())
        }
        OutputFormat::Toml => {
            let final_path = environment.result()?.working_dir().join(path);

            fs::write(&final_path, &environment.to_toml()?)?;
            let label = cformat!("<bold><green>✔Snapshot</>");
            println!("{label}\n Saved TOML snapshot to {}", final_path.display());

            Ok(())
        }
    }
}
