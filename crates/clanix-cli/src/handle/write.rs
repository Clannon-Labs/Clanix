use std::fs;
use std::path::PathBuf;

use clanix::Environment;
use color_print::cformat;

use crate::NAME;
use crate::error::Result;

pub fn handle_write(
    environment: &mut Environment,
    verbose: bool,
    path: PathBuf,
    content: String,
) -> Result<()> {
    let result = environment.result()?;

    let final_path = result.working_dir().join(path);
    fs::write(&final_path, &content)?;

    if verbose {
        println!("{environment}");
    } else {
        let label = cformat!("<bold><green>✔{NAME}</>");
        let simulated = cformat!("<yellow>[simulated]</>");
        println!("{label}  {simulated} Wrote to {}", final_path.display());
    }

    let result = environment.result_mut()?;

    let _ = result.set_success(true);
    let _ = result.set_status(Some(101));
    result.set_stderr("".to_string())?;

    Ok(())
}
