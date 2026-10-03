
use clanix::{
    Config,
    ExecutionResult,
};

use crate::NAME;
use crate::ConfigCommands;

use crate::error::Result;

pub fn handle_config(
    result: &mut ExecutionResult,
    configcommand: ConfigCommands,
    verbose: bool,
) -> Result<()>{
    /*
        * The core could technically be able to set the config to default if
        * we called .set_default() at the end of the chain but it would be 
        * a waste of resources, so we check the condition and set to default if
        * user wants so
        */
    
    // println!("{disable_timeout:?}, {timeout_ms:?}, {working_dir:?}");
    match configcommand {
        ConfigCommands::Default => {
            let config = Config::set_default()?;

            // println!("{config:?}");
            if verbose {
                let message = format!("{NAME}: Successfully set the following config:\n{config:?}\n");
                println!("{message}");
                let _ = result.set_stdout(message)?;
            } else {
                let message = format!("{NAME}: Successfully set the config.\n");
                println!("{message}");
                let _ = result.set_stdout(message)?;
            }
        },
        ConfigCommands::Init => {
            println!("Init is not developed yet!")
        },
        ConfigCommands::Set {disable_timeout, timeout_ms, working_dir} => {
            let config = Config::read()?
                .set_disable_timeout(disable_timeout)?
                .set_timeout_ms(timeout_ms)?
                .set_working_dir(working_dir);
            
            if verbose {
                let message = format!("{NAME}: Successfully set the following config:\n{config:?}\n");
                println!("{message}");
                let _ = result.set_stdout(message)?;
            } else {
                let message = format!("{NAME}: Successfully set the config.\n");
                println!("{message}");
                let _ = result.set_stdout(message)?;
            }
        },
        ConfigCommands::What{disable_timeout, timeout_ms, working_dir} => {
            let config = Config::read()?;
            
            if disable_timeout {
                println!("{:?}", config.disable_timeout())
            } else if timeout_ms {
                println!("{:?}", config.timeout_ms())
            } else if working_dir {
                println!("{:?}", config.working_dir())
            } else {
                println!("{:?}", config)
            }
        }
    }

    let _ = result.set_success(true)?;
    let _ = result.set_status(Some(101))?;
    let _ = result.set_stderr("".to_string())?;
    Ok(())
}
