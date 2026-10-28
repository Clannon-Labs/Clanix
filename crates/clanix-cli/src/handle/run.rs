use std::borrow::Cow;

use clanix::{Environment, ExecutionOutput};
use color_print::cformat;

use crate::NAME;
use crate::error::Result;

pub fn handle_run(
    environment: &mut Environment,
    program: String,
    args: Vec<String>,
    verbose: bool,
    disable_timeout: bool,
    timeout: Option<u64>,
    working_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    // Run in that environment with these arguments
    let request = environment
        .new_request(&program)?
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

    let mut success = false;
    let mut output_result = Cow::Borrowed("");
    
    match result.output() {
        ExecutionOutput::Separate { stdout, stderr } => {
            let stdout_output = String::from_utf8_lossy(stdout);
            let stderr_output = String::from_utf8_lossy(stderr);
        
            // We display stdout incase of success and stderr incase of failure
            // But some commands might run successfully but put the content in stderr
            // so we check if there's anything on stderr incase we find stdout empty
            // even incase of success
            if result.success() {
                output_result = if !stdout_output.trim().is_empty() {
                    stdout_output
                } else {
                    stderr_output
                };

                success = true;
        
                // let _ = environment.result_mut()?
                // .set_stdout(message)?;
            } else {
                output_result = if !stderr_output.trim().is_empty() {
                    stderr_output
                } else {
                    stdout_output
                };

                success = false;
                
                // let final_output = output.clone().into_owned();
        
                // let _ = environment.result_mut()?
                // .set_stderr(final_output)?;
            }
        },
        ExecutionOutput::Merged { output } => {
            output_result = String::from_utf8_lossy(output);

            // Marking success as true because until now, there's no way to actually
            // know whether or not the process succeded
            success = true;
        },
        ExecutionOutput::Terminal => {
            // The process already interacted through the terminal session.
            // Don't try to print output from the result again
        },
        _ => {}
    };

    let final_output = output_result.clone().into_owned();

    if success {
        let label = cformat!("<bold><green>✔{NAME}</>");
        if verbose {
            println!("{label}\n\n{environment}");
        } else {
            println!("{label}  {program} {args} completed successfully.\n\n {final_output}\n");
        }
    } else {
        if verbose {
            println!("{environment}");
        } else {
            let label = cformat!("<bold><red> {NAME}</>");
            println!("{label}  {program} {args} failed.\n\n {output_result}\n");
        }
    }

    Ok(())
}
