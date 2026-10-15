
use clanix::{Environment, ExecutionResult};

use crate::NAME;
use crate::error::Result;

// This currently doesn't return any error because I am not checking
// for potential errors with the program and args,
// 
// But the error types needed for it are already made in crate::error
pub fn handle_run(
    environment: &mut Environment,
    program: String,
    args: Vec<String>,
    verbose: bool,
    disable_timeout: bool,
    timeout: Option<u64>,
    working_dir: Option<std::path::PathBuf>
) -> Result<ExecutionResult> {

    // Run in that environment with these arguments
    let request = environment.new_request(&program)?
            .args(&args)
            .set_disable_timeout(disable_timeout)?
            .set_timeout_s(timeout)?
            .set_working_dir(working_dir)?;
        
    // IMPORTANT: Execute the request in that environment because
    // running the request outside of the environment will make the
    // request and error drift apart from the rest of the environment..
    // 
    let _ = environment.execute_request(request)?;
    
    // Get the result from the environment and replace the old default result
    let result = environment.result()?.to_owned();
        
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

        let message = if verbose {
                environment.to_string()
            } else {
                format!("{NAME}: {program} {args} was successful!\n{final_output}\n")
            };

        println!("{message}");

        let _ = environment.result_mut()?
            .set_stdout(message)?;
        
    } else {
        let output = if !stderr_output.trim().is_empty() {
            stderr_output
        } else {
            stdout_output
        };

        let final_output = output.clone().into_owned();

        let message = if verbose {
            environment.to_string()
        } else {
            format!("{NAME}: {program} {args} failed!\n{output}\n")
        };

        println!("{message}");

        let _ = environment.result_mut()?
            .set_stderr(final_output)?;
    }

    Ok(result)

}
