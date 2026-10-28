use clanix::{Config, Environment};
use color_print::cformat;

use crate::ConfigCommands;
use crate::NAME;

use crate::error::Result;

pub fn handle_config(
    environment: &mut Environment,
    configcommand: ConfigCommands,
    verbose: bool,
) -> Result<()> {
    /*
     * The core could technically be able to set the config to default if
     * we called .set_default() at the end of the chain but it would be
     * a waste of resources, so we check the condition and set to default if
     * user wants so
     */

    // println!("{disable_timeout:?}, {timeout_s:?}, {working_dir:?}");
    match configcommand {
        ConfigCommands::Set {
            default,
            disable_timeout,
            timeout_s,
            working_dir,
        } => {
            if default {
                let _ = environment.set_default_config()?;
            } else {
                let _ = Config::read()?
                    .set_disable_timeout(disable_timeout)?
                    .set_timeout_s(timeout_s)?
                    .set_working_dir(working_dir);
            }

            if verbose {
                println!("{environment}");
            } else {
                let label = cformat!("<bold><green> {NAME}</>");
                println!("{label}  Configuration updated.\n");
            }

            // let _ = environment.result_mut()?
            // .set_stdout(message)?;
        }
        ConfigCommands::What {
            disable_timeout,
            timeout_s,
            working_dir,
        } => {
            let config = Config::read()?;

            if disable_timeout {
                println!("{}", config.disable_timeout())
            } else if timeout_s {
                println!("{:?}", config.timeout_s())
            } else if working_dir {
                println!("{}", config.working_dir().display())
            } else {
                let heading = cformat!("<bold><cyan>Clanix configuration</>");
                println!("{heading}\n{config}");
            }
        }
    }

    let result = environment.result_mut()?;

    let _ = result.set_success(true);
    let _ = result.set_status(Some(101));
    result.set_stderr("".to_string())?;
    Ok(())
}
