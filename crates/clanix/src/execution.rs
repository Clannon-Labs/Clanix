
use std::io::Read;
use std::sync::mpsc;
use std::borrow::Cow;
use std::time::{Instant, Duration};
use std::path::PathBuf;
use std::ffi::OsString;

use std::{
    process::{
        Command,
        Stdio,
    }
};

use uuid::Uuid;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

use crate::Policy;
use crate::shared::PolicyTrait;
use crate::config::Config;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    id: Uuid,
    success: bool,
    status: Option<i32>,

    #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
    stdout: Vec<u8>,

    #[serde(serialize_with = "bytes_to_str", deserialize_with = "str_to_bytes")]
    stderr: Vec<u8>,
    
    working_dir: PathBuf,
}

fn bytes_to_str<S>(bytes: &[u8], serializer: S) -> Result<S:: Ok, S::Error>
    where S: Serializer,
    {
        serializer.serialize_str(
            &String::from_utf8_lossy(bytes)
        )
    }

fn str_to_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.into_bytes())
    }

    impl ExecutionRequest {
        pub fn new<S: Into<OsString>>(program: S) -> ExecutionRequestBuilder {
            // This just gives you request builder with the
            // default message and you have to set the program/args
            // to actually execute it
            ExecutionRequestBuilder {
                program: program.into(),
                args: Vec::new(),
                policy: Policy::defaults()
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
    
            let working_dir = &self.policy.working_dir;
            let timeout_ms = self.policy.timeout_ms;
            let disable_timeout = self.policy.disable_timeout;
    
            // println!("Current working directory is {:?}", &working_dir);
            // println!("Timeout_ms: {:?}", timeout_ms);
            // println!("Timeout disable: {}", disable_timeout);
            
            let mut child = Command::new(&self.program)
                .args(&self.args)
                .current_dir(working_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("Couldn't spawn a child process");
    
            // println!("Just ran the command");
            
            let start_time = Instant::now();
            let id = Uuid::new_v4();
    
            // println!("Just set the start time and id");
    
            let mut stdout_stream = child.stdout.take().expect("Couldn't get the stdout stream!");
            let mut stderr_stream = child.stderr.take().expect("Couldn't get the stderr stream!");
    
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
                working_dir: self.policy.working_dir.clone(),
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
        
        // This is used to get the policy so you can later set
        // the individual policy key
        // These functions are for setting the individual
        // policy keys
        pub fn set_timeout_ms(mut self, timeout_ms: Option<u64>) -> Self {
            self.policy = Policy::set_timeout_ms(self.policy, timeout_ms);
            self
        }
    
        pub fn set_disable_timeout(mut self, disable: bool) -> Self {
            self.policy = Policy::set_disable_timeout(self.policy, disable);
            self
        }
        
        pub fn set_working_dir(mut self, directory: Option<PathBuf>) -> Self {
            self.policy = Policy::set_working_dir(self.policy, directory);
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
    
        pub fn defaults(working_dir: &Option<PathBuf>) -> Self {
            /*
             * If user/client gave working dir, then use it for this run,
             * if not, read the working dir set in config and put that as default 
             * for this run
             */
            let directory = match working_dir {
                Some(dir) => dir.clone(),
                None => {
                    Config::read().working_dir().clone() // I gotta do something about these clones man..
                }
            };
            
            Self {
                id: Uuid::new_v4(),
                success: false,
                status: None,
                stdout: vec![],
                stderr: vec![],
                working_dir: directory
            }
        }
    
        pub fn id(&self) -> &Uuid {
            &self.id
        }
    
        pub fn working_dir(&self) -> &PathBuf {
            &self.working_dir
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
    
        pub fn set_success(&mut self, new_success: bool) {
            self.success = new_success
        }
    
        pub fn set_status(&mut self, new_status: Option<i32>) {
            self.status = new_status
        }
        
        pub fn set_stdout(&mut self, new_stdout: String) {
            self.stdout = new_stdout.into_bytes()
        }
    
        pub fn set_stderr(&mut self, new_stderr: String) {
            self.stderr = new_stderr.into_bytes()
        }
    }
