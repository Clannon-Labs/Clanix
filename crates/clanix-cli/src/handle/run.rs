use std::io::{self, Write};
use std::borrow::Cow;
use std::time::Duration;

use clanix::{Environment, ExecutionOutput, ExecutionEvent};
use color_print::cformat;

use crate::NAME;
use crate::error::Result;

pub fn handle_run(
    environment: &mut Environment,
    program: String,
    args: Vec<String>,

    stream: bool,
    verbose: bool,
    disable_timeout: bool,
    timeout: Option<Duration>,
    working_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    // Run in that environment with these arguments
    let request = environment
        .new_request(&program)?
        .args(&args)
        .set_disable_timeout(disable_timeout)?
        .set_timeout(timeout)?
        .set_working_dir(working_dir)?;

    // IMPORTANT: Execute the request in that environment because
    // running the request outside of the environment will make the
    // request and error drift apart from the rest of the environment..
    //
    // let result = environment.execute_request(request)?.to_owned();

    let mut error: Option<io::Error> = None;
    
    let result = if !stream {
        environment.execute_request(request)?.to_owned()
    } else {
          environment.execute_request_with_events(request, |event| {
              match event {
                  #[allow(unused)]
                  ExecutionEvent::Started { id } => {
                      // We could later validate that clanix is giving us the
                      // same id across the byte chunks
                  },
                  ExecutionEvent::Output { bytes, .. } => {
                      // Write to stdout instead of printing
                      if error.is_none() {
                          let mut stdout = io::stdout().lock();

                          if let Err(err) = stdout.write_all(&bytes)
                              .and_then(|()| stdout.flush()) {
                                  error = Some(err);
                              }
                      }                 
                  }
              }
          })?.to_owned()
    };

    if let Some(err) = error {
        return Err(err.into());
    }
    
    // Get the result from the environment and replace the old default result
    // let result = environment.result()?.to_owned();

    let args = args.join(" ");

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
        
                // let _ = environment.result_mut()?
                // .set_stdout(message)?;
            } else {
                output_result = if !stderr_output.trim().is_empty() {
                    stderr_output
                } else {
                    stdout_output
                };
                
                // let final_output = output.clone().into_owned();
        
                // let _ = environment.result_mut()?
                // .set_stderr(final_output)?;
            }
        },
        ExecutionOutput::Merged { .. } => {
            // The output has already been printed to the terminal, no
            // need to print it again

        },
        ExecutionOutput::Terminal => {
            // The process already interacted through the terminal session.
            // Don't try to print output from the result again

        },
        _ => {}
    };

    let final_output = output_result.clone().into_owned();

    if result.success() {
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
