
use clanix::ExecutionResult;

use crate::NAME;

pub fn handle_test(result: &mut ExecutionResult, verbose: bool) {
    if verbose {
        let message = format!("{NAME} [SIMULATED]: Request to run tests was successful\n");
        println!("{message}");
        result.set_stdout(message);
    } else {
        let message = "[SIMULATED]: Tests successful!".to_string();
        println!("{message}");
        result.set_stdout(message);
    }

    result.set_success(true);
    result.set_status(Some(101));
    result.set_stderr("".to_string());
}