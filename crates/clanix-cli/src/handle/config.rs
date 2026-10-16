
use clanix::{
    Config,
    Environment,
};

use crate::NAME;
use crate::ConfigCommands;

use crate::error::Result;

pub fn handle_config(
    environment: &mut Environment,
    configcommand: ConfigCommands,
    verbose: bool,
) -> Result<()>{
    /*
        * The core could technically be able to set the config to default if
        * we called .set_default() at the end of the chain but it would be 
        * a waste of resources, so we check the condition and set to default if
        * user wants so
        */
    
    // println!("{disable_timeout:?}, {timeout_s:?}, {working_dir:?}");
    match configcommand {
        ConfigCommands::Set {default, disable_timeout, timeout_s, working_dir} => {
            if default {
                let _ = environment.set_default_config()?;
                
            } else {
                let _ = Config::read()?
                    .set_disable_timeout(disable_timeout)?
                    .set_timeout_s(timeout_s)?
                    .set_working_dir(working_dir);
            }
            
            let message = if verbose {
                environment.to_string()
            } else {
                format!("{NAME}: Successfully set the config.\n")
            };

            println!("{message}");

            // let _ = environment.result_mut()?
                // .set_stdout(message)?;
        },
        ConfigCommands::What{disable_timeout, timeout_s, working_dir} => {
            let config = Config::read()?;
            
            if disable_timeout {
                println!("{}", config.disable_timeout())
            } else if timeout_s {
                println!("{:?}", config.timeout_s())
            } else if working_dir {
                println!("{}", config.working_dir().display())
            } else {
                println!("{}", config)
            }
        }
    }

    let result = environment.result_mut()?;
        
    let _ = result.set_success(true)?;
    let _ = result.set_status(Some(101))?;
    let _ = result.set_stderr("".to_string())?;
    Ok(())
}
