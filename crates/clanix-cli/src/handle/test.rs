use clanix::Environment;
use color_print::cformat;

use crate::NAME;

use crate::error::Result;

pub fn handle_test(environment: &mut Environment, verbose: bool) -> Result<()> {
    if verbose {
        println!("{environment}");
    } else {
        let label = cformat!("<bold><green>✔{NAME}</>");
        let simulated = cformat!("<yellow>[simulated]</>");
        println!("{label}  {simulated} Tests completed successfully.");
    }

    let result = environment.result_mut()?;

    let _ = result.set_success(true);
    let _ = result.set_status(Some(101));
    result.set_stderr("".to_string())?;

    Ok(())
}
