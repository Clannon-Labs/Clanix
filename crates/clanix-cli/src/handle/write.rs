
use std::fs;
use std::path::PathBuf;

use clanix::Environment;

use crate::NAME;
use crate::error::Result;

pub fn handle_write(
    environment: &mut Environment,
    verbose: bool, 
    path: PathBuf, 
    content: String
) -> Result<()> {

    let result = environment.result()?;
        
    let final_path = result.working_dir().join(path);
    fs::write(&final_path, &content)?; 

    let message = if verbose {
        environment.to_string()
    } else {
        format!("{NAME}: [SIMULATED]: Wrote to {final_path:?}")
    };

    println!("{message}");

    let result = environment.result_mut()?;

    let _ = result.set_success(true)?;
    let _ = result.set_status(Some(101))?;
    let _ = result.set_stderr("".to_string())?;

    Ok(())
}