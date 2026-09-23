
use std::env;
use std::path::PathBuf;
use std::ffi::OsString;
use std::io::Read;
use std::borrow::Cow;
use std::sync::mpsc;
use std::time::{Instant, Duration};
use std::{
    process::{
        Command,
        Stdio,
    },
};

use uuid::Uuid;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    program: OsString,
    args: Vec<OsString>,
    policy: Policy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequestBuilder {
    program: OsString,
    args: Vec<OsString>,
    policy: Policy,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    id: Uuid,
    success: bool,
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    timeout_ms: Duration,
    working_dir: PathBuf,
}

/*
 * The defaults are currently hardcoded,
 * but we can change it later to clean things up
 */
impl Policy {
    fn default() -> Self {
        Self {
            timeout_ms: Self::default_timeout_ms(),
            working_dir: Self::default_working_dir(), 
        }
    }

    fn default_timeout_ms() -> Duration {
        Duration::from_millis(100)
    }

    fn default_working_dir() -> PathBuf {
        env::current_dir().unwrap() // Replace this unwrap() later
    }
}

impl ExecutionRequest {
    pub fn new<S: Into<OsString>>(program: S) -> ExecutionRequestBuilder {
        // This just gives you request builder with the
        // default message and you have to set the program/args
        // to actually execute it
        ExecutionRequestBuilder {
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default()
        }
    }   
}

impl ExecutionRequestBuilder {
    // It consumes the request after you call execute,
    // so you have to create a new request if you want
    // to call execute again
    pub fn execute(&self) -> Result<ExecutionResult, String> {
        /*
         * Change all the panic creating .expect()s with proper
         * error handling later
         */
        
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Couldn't spawn a child process");

        let timeout_ms = self.policy.timeout_ms;
        println!("Current working directory is {:?}", self.policy.working_dir);
        // println!("Timeout_ms: {:?}", timeout_ms);
        
        let start_time = Instant::now();
        let id = Uuid::new_v4();

        let mut stdout_stream = child.stdout.take().expect("Couldn't get the stdout stream!");
        let mut stderr_stream = child.stderr.take().expect("Couldn't get the stderr stream!");
        
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        
        std::thread::spawn(move || {
            let mut buffer = [0u8; 1024]; // 1KB buffer
            while let Ok(bytes_read) = stdout_stream.read(&mut buffer){
                if bytes_read == 0 {break;}
                if tx.send(buffer[..bytes_read].to_vec()).is_err() {break;}
            }
        });

        let mut captured_stdout: Vec<u8> = Vec::new();

        // keep checking the incoming (non blocking) bytes in loop
        loop {
            let elapsed_time = start_time.elapsed();
            // if the process has ran for already over the timeout,
            // kill it before you do anything else
            if elapsed_time >= timeout_ms {
                let _ = child.kill();
                println!(
                  "Process ran for {:?} while the timeout was {:?}",
                elapsed_time, timeout_ms
                );
                break;
            }

            match rx.try_recv() {
                Ok(bytes) => captured_stdout.extend(bytes),
                Err(mpsc::TryRecvError::Empty) => {
                    // No bytes to read.. EOF maybe??
                    // Sleep for 100 micro s and check again
                    std::thread::sleep(Duration::from_micros(100))
                },
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Process exited.. it finished maybe??
                    break;
                }
            }
        }

        let exit_status = child.wait().expect("Couldn't get exit status from the process");

        // Read the stderr if there is any
        let mut captured_stderr: Vec<u8> = Vec::new();
        let _ = stderr_stream.read_to_end(&mut captured_stderr);

        Ok(ExecutionResult {
            id,
            success: exit_status.success(),
            status: exit_status.code(),
            stdout: captured_stdout,
            stderr: captured_stderr,
        })
    }

    // The new() function already has gets the program,
    // so we dont need another program() here
    // This is used to pass just one argument
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
    
    // This is used to get the policy so you can later set
    // the individual policy key
    // These functions are for setting the individual
    // policy keys
    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.policy.timeout_ms = Duration::from_millis(timeout_ms);
        self
    }

    pub fn directory(mut self, directory: Option<PathBuf>) -> Self {
        self.policy.working_dir = directory.unwrap_or_else(
            || Policy::default_working_dir()
        );
        self
    }
}

impl ExecutionResult {
    // These simple methods exist for the caller to call,
    // because the internal structure might change and
    // if the caller calls the internal elements directly,
    // it could break,
    // so calling these methods is safer because these will
    // translate whatever was changed internally into what the
    // callers expect; with no breaking changes
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    pub fn success(&self) -> bool {
        self.success
    }
    
    pub fn status(&self) -> Option<i32> {
        self.status
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stdout_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub fn stderr(&self) -> &[u8]  {
        &self.stderr
    }

    pub fn stderr_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    // These functions might not be needed maybe??
    // Like why do they need to set the original stdout/stderr??
    // 
    // if they want to modify it, they can copy and delete/concat
    // stuff.. so i might delete it soon
    pub fn set_stdout(&mut self, new_stdout: String) {
        self.stdout = new_stdout.into_bytes()
    }

    pub fn set_stderr(&mut self, new_stderr: String) {
        self.stderr = new_stderr.into_bytes()
    }
}
