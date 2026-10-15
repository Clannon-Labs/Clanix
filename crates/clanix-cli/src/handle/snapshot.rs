
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
            let final_path = environment.result()?
                .working_dir().join(path);
            
            fs::write(&final_path, &environment.to_string())?;
            println!("Wrote stdout to {final_path:?}");
            
            Ok(())
        },
        OutputFormat::Json => {            
            let final_path = environment.result()?
                .working_dir().join(path);
            
            fs::write(&final_path, &environment.to_json()?)?;
            println!("Wrote stdout to {final_path:?}");
            
            Ok(())
        }
        OutputFormat::Toml => {            
            let final_path = environment.result()?
                .working_dir().join(path);
            
            fs::write(&final_path, &environment.to_toml()?)?;
            println!("Wrote stdout to {final_path:?}");
            
            Ok(())
        }
    }
}