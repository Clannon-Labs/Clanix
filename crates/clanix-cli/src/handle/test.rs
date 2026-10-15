
use clanix::Environment;

use crate::NAME;

use crate::error::Result;

pub fn handle_test(
    environment: &mut Environment,
    verbose: bool,
) -> Result<()> {
    let message = if verbose {
        environment.to_string()
    } else {
        format!("{NAME}: [SIMULATED]: Tests successful!")
    };

    println!("{message}");

    let result = environment.result_mut()?;
    
    let _ = result.set_success(true)?;
    let _ = result.set_status(Some(101))?;
    let _ = result.set_stderr("".to_string())?;

    Ok(())
}