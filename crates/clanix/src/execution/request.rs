
use std::io::Read;
use std::sync::mpsc;
use std::path::PathBuf;
use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::time::{Instant, Duration};

use uuid::Uuid;
use serde::{Serialize, Deserialize};

use crate::Policy;

use super::ExecutionResult;
use crate::RequestError;
use crate::Result;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) policy: Policy,
}

impl std::fmt::Display for ExecutionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nExecution Request")?;
        writeln!(f, "--------\n")?;
        writeln!(f, "Program: {}", self.program.display())?;

        if !self.args.is_empty() {
            // This {:#?} should pretty print, maybe??
            writeln!(f, "Arguments: {:#?}", self.args)?;
        }

        // We keep policy string blank because Policy itself implements
        // its Display version
        writeln!(f, "{}\n", self.policy)?;
        
        Ok(())
    }
}

impl ExecutionRequest {
    pub fn default() -> Result<Self> {
        Ok(Self {
            program: OsString::new(),
            args: Vec::new(),
            policy: Policy::default()?,
        })
    }

    // It consumes the request after you call execute,
    // so you have to create a new request if you want
    // to call execute again
    pub(crate) fn execute(&self) -> Result<ExecutionResult> {
        /*
            * Change all the panic creating .expect()s with proper
            * error handling later
            */
        
        let working_dir = &self.policy.working_dir();
        let timeout_ms = self.policy.timeout_ms();
        let disable_timeout = self.policy.disable_timeout();

        // println!("Current working directory is {:?}", &working_dir);
        // println!("Timeout_ms: {:?}", timeout_ms);
        // println!("Timeout disable: {}", disable_timeout);
        
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| RequestError::SpawnError)?;

        // println!("Just ran the command");
        
        let start_time = Instant::now();
        let id = Uuid::new_v4();

        // println!("Just set the start time and id");

        let mut stdout_stream = child.stdout.take()
            .ok_or_else(|| RequestError::StdoutError)?;
        
        let mut stderr_stream = child.stderr.take()
            .ok_or_else(|| RequestError::StderrError)?;

        // println!("Just took stdout and stderr streams");
        
        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        // println!("Just created a channel for Vec<u8>");
        
        std::thread::spawn(move || {
            let mut buffer = [0u8; 1024]; // 1KB buffer            
            while let Ok(bytes_read) = stdout_stream.read(&mut buffer){
                if bytes_read == 0 {break;}
                if tx.send(buffer[..bytes_read].to_vec()).is_err() {
                    // println!("Just sent some bytes to buffer");
                    break;
                }
            }
        });

        let mut captured_stdout: Vec<u8> = Vec::new();

        // keep checking the incoming (non blocking) bytes in loop
        loop {
            let elapsed_time = start_time.elapsed();
            // if the process has ran for already over the timeout,
            // kill it before you do anything else
            // println!("About to check disable_timeout: {} and timeout_ms: {:?}", disable_timeout, timeout_ms);
            if (disable_timeout == false) && elapsed_time >= timeout_ms {
                let _ = child.kill();
                println!(
                    "Process ran for {:?} while the timeout was {:?}",
                    elapsed_time, timeout_ms
                );
                break;
            }

            match rx.try_recv() {
                Ok(bytes) => {
                    captured_stdout.extend(bytes);
                    // println!("Just extended captured_stdout with some bytes")
                },
                Err(mpsc::TryRecvError::Empty) => {
                    // No bytes to read.. EOF maybe??
                    // Sleep for 100 micro s and check again
                    std::thread::sleep(Duration::from_micros(100))
                },
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Process exited.. it finished maybe??
                    // println!("Process exited");
                    break;
                }
            }
        }

        let exit_status = child.wait()
            .map_err(|_| RequestError::StatusError)?;

        // Read the stderr if there is any
        let mut captured_stderr: Vec<u8> = Vec::new();
        let _ = stderr_stream.read_to_end(&mut captured_stderr);

        Ok(ExecutionResult {
            id,
            success: exit_status.success(),
            status: exit_status.code(),
            stdout: captured_stdout,
            stderr: captured_stderr,
            working_dir: self.policy.working_dir().to_path_buf()
        })
    }
    
    pub fn arg<S: Into<OsString>>(mut self, arg: S) -> Self {
        self.args.push(arg.into());
        self
    }

    // This is used to pass multiple arguments or a Vec< >
    pub fn args<I, S>(mut self, args: I) -> Self 
        where
            I: IntoIterator<Item = S>,
            S: Into<OsString>,
        {
            for arg in args {
                self.args.push(arg.into());
            }
            self
        }

    /*
     * These functions return Error(which gets converted from PolicyError internally) 
     * instead of RequestError because these are used to change the policy state and
     * not to actually request the execution.
     * 
     * Also as you are seeing, the method used to actually request execution (execute())
     * returns RequestError wrapped in the Error
     */
        
    // This is used to get the policy so you can later set
    // the individual policy key
    // These functions are for setting the individual
    // policy keys
    pub fn set_timeout_ms(mut self, timeout_ms: Option<u64>) -> Result<Self> {
        self.policy = Policy::set_timeout_ms(self.policy, timeout_ms)?;        
        Ok(self)
    }

    pub fn set_disable_timeout(mut self, disable: bool) -> Result<Self> {
        self.policy = Policy::set_disable_timeout(self.policy, disable)?;
        Ok(self)
    }
    
    pub fn set_working_dir(mut self, directory: Option<PathBuf>) -> Result<Self> {
        self.policy = Policy::set_working_dir(self.policy, directory)?;
        Ok(self)
    }
}
