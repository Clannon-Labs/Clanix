
use clanix::ExecutionResult;

use crate::NAME;
use crate::error::Result;

// This currently doesn't return any error because I am not checking
// for potential errors with the program and args,
// 
// But the error types needed for it are already made in crate::error
pub fn handle_run(
    result: &mut ExecutionResult,
    program: String,
    args: Vec<String>,
    verbose: bool) -> Result<()> {
    let args = args.join(" ");
    
    let stdout_output = &result.stdout_string();
    let stderr_output = &result.stderr_string();

    // We display stdout incase of success and stderr incase of failure
    // But some commands might run successfully but put the content in stderr
    // so we check if there's anything on stderr incase we find stdout empty 
    // even incase of success
    if result.success() {
        let output = if !stdout_output.trim().is_empty() {
            stdout_output
        } else {
            stderr_output
        };
        
        let final_output = output.clone().into_owned();
        if verbose {
            let message = format!("{NAME}: Command {program} {args} was ran successfully!\n{final_output}\n");
            println!("{message}");
            let _ = result.set_stdout(message)?;
        } else {
            let message = format!("{NAME}: {program} {args} was successful!\n{final_output}\n");
            println!("{message}");
            let _ = result.set_stdout(message)?;
        }
    } else {
        let output = if !stderr_output.trim().is_empty() {
            stderr_output
        } else {
            stdout_output
        };

        let final_output = output.clone().into_owned();
        if verbose {
            let message = format!("{NAME}: Command {program} {args} failed!\n{output}\n");
            eprintln!("{message}");
            let _ = result.set_stderr(final_output)?;
        } else {
            let message = format!("{NAME}: {program} {args} failed!\n{output}\n");
            eprintln!("{message}");
            let _ = result.set_stderr(final_output)?;
        }
    }

    Ok(())

}
