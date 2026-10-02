
use std::fs;
use std::path::PathBuf;

use clanix::ExecutionResult;

use crate::OutputFormat;
use crate::error::Result;


pub fn handle_snapshot(
    result: &mut ExecutionResult,
    path: PathBuf,
    output_format: OutputFormat
) -> Result<()> {
    match output_format {
        // No verbose check because inside the match command block,
        // each command already wrote their verbose output
        OutputFormat::Text => {
            let final_path = result.working_dir().join(path);
            fs::write(&final_path, &result.stdout()).unwrap();
            println!("Wrote stdout to {final_path:?}");
            Ok(())
        },
        OutputFormat::Json => {
            let final_path = result.working_dir().join(path);
            let output = serde_json::to_string_pretty(&result).unwrap();
            fs::write(&final_path, &output).unwrap();
            println!("Wrote stdout to {final_path:?}");
            Ok(())
        }
        OutputFormat::Toml => {
            let final_path = result.working_dir().join(path);
            let output = toml::to_string_pretty(&result).unwrap();
            fs::write(&final_path, &output).unwrap();
            println!("Wrote stdout to {final_path:?}");
            Ok(())
        }
    }
}