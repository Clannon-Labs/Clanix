
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
    program: String,
    args: Vec<String>,
    policy: Policy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequestBuilder {
    program: String,
    args: Vec<String>,
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
}

/*
 * The defaults are currently hardcoded,
 * but we can change it later to clean things up
 */
impl Policy {
    fn default() -> Self {
        Self {
            timeout_ms: Duration::from_millis(1),
        }
    }
}

impl ExecutionRequest {
    pub fn new(program: String, args: Vec<String>) -> ExecutionRequestBuilder {
        ExecutionRequestBuilder {
            program: program,
            args: args,
            policy: Policy::default()
        }
    }   
}

impl ExecutionRequestBuilder {
    // It consumes the request after you call execute,
    // so you have to create a new request if you want
    // to call execute again
    pub fn execute(self) -> Result<ExecutionResult, String> {
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
            let timeout_ms = self.policy.timeout_ms;
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

        Ok(ExecutionResult{
            id,
            success: exit_status.success(),
            status: exit_status.code(),
            stdout: captured_stdout,
            stderr: captured_stderr,
        })
    }
    
    pub fn program(mut self, program: String) -> Self {
        self.program = program;
        self
    }

    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
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

    // Doing .clone() for these 2 functions for now
    // but i might have to figure out a way to avoid
    // this
    pub fn set_stdout(&mut self, new_stdout: String) {
        self.stdout = new_stdout.into_bytes()
    }

    pub fn set_stderr(&mut self, new_stderr: String) {
        self.stderr = new_stderr.into_bytes()
    }
}
