
use std::fs;
use std::path::PathBuf;

use clanix::ExecutionResult;

use crate::NAME;
use crate::error::Result;

pub fn handle_write(
    result: &mut ExecutionResult, 
    verbose: bool, 
    path: PathBuf, 
    content: String
) -> Result<()> {
    let final_path = result.working_dir().join(path);
    fs::write(&final_path, &content)?; 

    if verbose {
        let message = format!(
            "{NAME} [SIMULATED]: Request to write the content '{content}' to {final_path:?} was successful\n"
        );
        println!("{message}");
        let _ = result.set_stdout(message)?;
    } else {
        let message = format!("[SIMULATED]: Wrote to {final_path:?}");
        println!("{message}");
        let _ = result.set_stdout(message)?;
    }

    let _ = result.set_success(true)?;
    let _ = result.set_status(Some(101))?;
    let _ = result.set_stderr("".to_string())?;

    Ok(())
}