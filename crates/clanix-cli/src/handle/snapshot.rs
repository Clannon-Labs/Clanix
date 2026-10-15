
use std::fs;
use std::path::PathBuf;

use clanix::Environment;

use crate::OutputFormat;
use crate::error::Result;


pub fn handle_snapshot(
    environment: &mut Environment,
    path: PathBuf,
    output_format: OutputFormat
) -> Result<()> {
    match output_format {
        // No verbose check because inside the match command block,
        // each command already wrote their verbose output
        OutputFormat::Text => {
            let result = environment.result()?;
            
            let final_path = result.working_dir().join(path);
            
            fs::write(&final_path, &result.stdout())?;
            println!("Wrote stdout to {final_path:?}");
            
            Ok(())
        },
        OutputFormat::Json => {
            let result = environment.result()?;
            
            let final_path = environment.result()?.working_dir().join(path);
            let output = result.to_json()?;
            
            fs::write(&final_path, &output)?;
            println!("Wrote stdout to {final_path:?}");
            
            Ok(())
        }
        OutputFormat::Toml => {
            let result = environment.result()?;
            
            let final_path = result.working_dir().join(path);
            let output = result.to_toml()?;
            
            fs::write(&final_path, &output)?;
            println!("Wrote stdout to {final_path:?}");
            
            Ok(())
        }
    }
}