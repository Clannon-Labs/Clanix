
use clanix::ExecutionResult;

use crate::NAME;

use crate::error::Result;

pub fn handle_test(result: &mut ExecutionResult, verbose: bool) -> Result<()> {
    if verbose {
        let message = format!("{NAME} [SIMULATED]: Request to run tests was successful\n");
        println!("{message}");
        let _ = result.set_stdout(message)?;
    } else {
        let message = "[SIMULATED]: Tests successful!".to_string();
        println!("{message}");
        let _ = result.set_stdout(message)?;
    }

    let _ = result.set_success(true)?;
    let _ = result.set_status(Some(101))?;
    let _ = result.set_stderr("".to_string())?;

    Ok(())
}