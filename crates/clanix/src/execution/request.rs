
use std::io::Read;
use std::sync::mpsc;
use std::path::PathBuf;
use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::time::{Instant, Duration};

use uuid::Uuid;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

use crate::Policy;

use super::ExecutionResult;
use crate::RequestError;
use crate::Result;

use crate::Error;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    #[serde(
        serialize_with = "serialize_os_string",
        deserialize_with = "deserialize_os_string"
    )]
    pub(crate) program: OsString,
    #[serde(
        serialize_with = "serialize_os_strings",
        deserialize_with = "deserialize_os_strings"
    )]
    pub(crate) args: Vec<OsString>,
    pub(crate) policy: Policy,
}

fn serialize_os_string<S>(
    value: &OsString,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.to_string_lossy().serialize(serializer)
    }

fn deserialize_os_string<'de, D> (
    deserializer: D
) -> std::result::Result<OsString, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(OsString::from)
    }

fn serialize_os_strings<S>(
    values: &[OsString],
    serializer: S
) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        values
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

fn deserialize_os_strings<'de, D>(
    deserializer: D
) -> std::result::Result<Vec<OsString>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<String>::deserialize(deserializer)
            .map(|values| values.into_iter()
                .map(OsString::from)
                .collect())
    }

impl std::fmt::Display for ExecutionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nExecution Request")?;
        writeln!(f, "--------\n")?;
        writeln!(f, "Program: {}", self.program.display())?;

        if !self.args.is_empty() {
            // This {:#?} should pretty print, maybe??
            writeln!(f, "Arguments: {:?}", self.args)?;
        }

        // We keep policy string blank because Policy itself implements
        // its Display version
        writeln!(f, "{}", self.policy)?;

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

    /// It consumes the request after you call execute,
    /// so you have to create a new request if you want
    /// to call execute again
    pub(crate) fn execute(&self) -> Result<ExecutionResult> {
        /*
            * Change all the panic creating .expect()s with proper
            * error handling later
            */

        let working_dir = &self.policy.working_dir();
        let timeout_s = self.policy.timeout_s();
        let disable_timeout = self.policy.disable_timeout();

        // println!("Current working directory is {:?}", &working_dir);
        // println!("Timeout_s: {:?}", timeout_s);
        // println!("Timeout disable: {}", disable_timeout);
        // println!("Executing: {self:#?}");

        let mut child = Command::new(&self.program)
            .args(&self.args)
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| RequestError::SpawnError {
                program: self.program.clone(),
                working_dir: working_dir.to_path_buf(),
                source,
            })?;

        
        // println!("Just ran the command");

        let start_time = Instant::now();
        let id = Uuid::new_v4();

        // println!("Just set the start time and id");

        let mut stdout_stream = child.stdout.take()
            .ok_or_else(|| RequestError::StdoutUnavailable)?;

        let mut stderr_stream = child.stderr.take()
            .ok_or_else(|| RequestError::StderrUnavailable)?;

        // println!("Just took stdout and stderr streams");

        let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>();

        // println!("Just created a channel for Vec<u8>");

        std::thread::spawn(move || {
            let mut buffer = [0u8; 1024];

            loop {
                match stdout_stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        if tx.send(Ok(buffer[..bytes_read].to_vec())).is_err() {
                            break;
                        }
                    },
                    Err(err) => {
                        // If error is occured, send the error and then break instead of 
                        // silently breaking
                        let _ = tx.send(Err(err));
                        break;
                    }
                }
            }
        });

        let mut captured_stdout: Vec<u8> = Vec::new();

        // keep checking the incoming (non blocking) bytes in loop
        loop {
            let elapsed_time = start_time.elapsed();
            // if the process has ran for already over the timeout,
            // kill it before you do anything else
            // println!("About to check disable_timeout: {} and timeout_s: {:?}", disable_timeout, timeout_s);
            if (disable_timeout == false) && elapsed_time >= timeout_s {
                child.kill()
                    .map_err(|e| RequestError::Kill(e))?;
                eprintln!(
                    "Process ran for {:?} while the timeout was {:?}",
                    elapsed_time, timeout_s
                );
                break;
            }

            match rx.try_recv() {
                Ok(Ok(bytes)) => {
                    captured_stdout.extend(bytes);
                    // println!("Just extended captured_stdout with some bytes")
                },
                Ok(Err(error)) => {
                    return Err(RequestError::StdoutRead(error).into());
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
            .map_err(RequestError::StatusError)?;

        // Read the stderr if there is any
        let mut captured_stderr: Vec<u8> = Vec::new();
        stderr_stream.read_to_end(&mut captured_stderr)
            .map_err(|e| RequestError::StderrRead(e))?;

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

    ///
    /// These functions return Error(which gets converted from PolicyError internally)
    /// instead of RequestError because these are used to change the policy state and
    /// not to actually request the execution.
    ///
    /// Also as you are seeing, the method used to actually request execution (execute())
    /// returns RequestError wrapped in the Error
    ///
    /// This is used to get the policy so you can later set
    /// the individual policy key
    /// These functions are for setting the individual
    /// policy keys
    pub fn set_timeout_s(mut self, timeout_s: Option<u64>) -> Result<Self> {
        self.policy = Policy::set_timeout_s(self.policy, timeout_s)?;
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

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self)
            .map_err(|e| Error::Request(RequestError::JsonError(e)))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(&self)
            .map_err(|e| Error::Request(RequestError::TomlError(e)))
    }
}
