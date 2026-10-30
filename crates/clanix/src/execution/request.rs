//! ExecutionRequest is the very first stage of running a process
//! 
//! You prepare a execution request, call .execute() and it hands you the
//! ExecutionResult if the process didn't timeout, and no errors occured
//! 
//! But if the process timedout, it will give you a Timeout Error with the
//! relevant information **Clanix** has by then.
//! 
//! Even though there isn't much information yet in the Timeout Error, in future
//! I will gather more information and return a rich Error instance instead of just
//! a plain error message

use std::io::Read;
use std::sync::mpsc;
use std::path::PathBuf;
use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::time::{Instant, Duration};

use uuid::Uuid;
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use os_pipe;

use crate::{ExecutionEvent, Policy};

use super::{ExecutionResult, ExecutionOutput};
use crate::RequestError;
use crate::Result;

use crate::Error;

#[derive(Debug)]
pub enum OutputMessage {
    Stdout(std::io::Result<Vec<u8>>),
    Stderr(std::io::Result<Vec<u8>>),
}

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
            writeln!(f, "Arguments: {:#?}", self.args)?;
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

    /// The method the clients can call if they want the output bytes to be streamed instead
    /// of returned when the process exits.
    /// 
    /// Example of how clients can use this method: 
    /// #[Example]
    /// 
    /// ```no_run
    /// // Give Clanix a closure (small function) that Clanix calls and hands you the id, bytes
    /// // through when it calculates them
    /// let result = request.execute_with_events(|event| {
    ///     // It's on the clients to choose what they want to do with the bytes they get
    ///     match event {
    ///         ExecutionEvent::Started { id } => {
    ///             // use the id
    ///         },
    ///         // We already have the id, so we dont need it again as both of them will be
    ///         // the same
    ///         ExecutionEvent::Output { bytes, .. } => {
    ///             // They could print it immediately because the bytes chunk may have a 
    ///             // partial line that needs to be printed before the other bytes
    ///             //
    ///             // But at the end of the day, it's upto them about what they want to do
    ///             // do with the bytes
    ///             print!("{}", String::from_utf8_lossy(&bytes));
    ///         },
    ///     }
    /// });
    /// ```
    /// 
    pub fn execute_with_events<F>(
        &self,
        mut on_event: F
    ) -> Result<ExecutionResult>
        where 
            F: FnMut(ExecutionEvent),
        {
            // Ask the OS for a connected reader/writer pair where the
            // bytes written in stdout_writer can be read from the reader
            let (reader, stdout_writer) =
                os_pipe::pipe().map_err(RequestError::MergedPipe)?;

            // Make another handle to the same stdout_writer pipe
            // 
            // Note that it doesn't copy the data or create another pipe,
            // we just get another handle to the same stdout_writer we have
            // 
            // This structure has one tradeoff tho; now we can't tell whether
            // the bytes came from stdout or stderr but it's exactly what the streaming
            // requires, we stream bytes to be displayed on the terminal
            // 
            // And most probably the order of the stdout/stderr bytes matters here
            // So we can't give stdout first and stderr later if the process had returned stderr
            // before
            // 
            // But we can't ensure that if we are reading stdout/stderr from different threads 
            // So we MUST read them together, even if that means we can no longer identify which byte
            // came from which writer
            let stderr_writer = stdout_writer
                .try_clone()
                .map_err(RequestError::MergedPipe)?;

            // Take the program, args and working dir we have on the ExecutionRequest and
            // give the stdout of the child process to the respective writers, so our reader
            // can read the bytes we are getting from the child process
            // 
            let mut child = Command::new(&self.program)
                .args(&self.args)
                .current_dir(self.policy.working_dir())
                .stdout(stdout_writer)
                .stderr(stderr_writer)
                .spawn()
                .map_err(|source| RequestError::SpawnError {
                    program: self.program.clone(),
                    working_dir: self.policy.working_dir().to_path_buf(),
                    source,
                })?;

            let start_time = Instant::now();
            let id = Uuid::new_v4();

            // Channel to transmit & receive the bytes
            let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>();

            // Spawn a thread where the reader can keep reading bytes and send the
            // bytes over to our main thread over the channel (which has rx as the receiver)
            // 
            // This allows the reader to keep reading without blocking our main thread
            // that is continuously inspecting the reader thread
            
            // Rust starts a new thread here and immediately gives the main thead a JoinHandle
            // stores in reader_threadthat the main thread (where the code above/below is running)
            // which it can use to inspect reader thread without blocking or being blocked
            let reader_thread = std::thread::spawn(move || {
                // Reader is moved into the thread because only that thread
                // is supposed to read from the pipe
                let mut reader = reader;

                // A buffer we read into before sending the bytes
                // 
                // This allows us to know what we have read and act based on it
                // (see the tx.send() line on Ok(bytes_read) block below)
                let mut buffer = [0u8; 1024]; 

                // Create an infinite loop to keep reading the bytes as they come
                loop {
                    // Try to read from the reader into our buffer
                    // This is a blocking operation which will wait/block until it
                    // reads the bytes from the reader, even if the reader gives
                    // it 0 bytes
                    match reader.read(&mut buffer) {
                        // Got 0 bytes, so maybe the reader has finished reading??
                        // So we dont need to read anymore, so break the loop
                        Ok(0) => break,

                        // We read some bytes, try sending it to the reader pipe
                        Ok(bytes_read) => {
                            // Try to send the bytes wrapped in Ok()
                            // But from the buffer we read to, send only the bytes which 
                            // we read this time, by copying them to a vector
                            // 
                            // But if this operation fails, break out of the loop
                            // Because the job of the reader thread is to read and send the bytes
                            // and if it can't to that, it should exit instead of running forever
                            // trying to do it everytime
                            if tx.send(
                                Ok(buffer[..bytes_read].to_vec())
                            ).is_err()
                            {
                                break;
                            }
                        },
                        // Couldn't read from the reader, report error to the receiver
                        // and exit the loop
                        Err(error) => {
                            let _ = tx.send(Err(error));
                            break;
                        }
                    }
                }
            });

            // Let the caller/client know that the execution has started and give
            // them the id they can use to track the execution
            // 
            // So they can distinguish between the process even if they launch many
            // of them, cuz each process would carry the same id up until the point
            // it finishes the stream; it WOULD NOT give a new id per byte chunk
            on_event(ExecutionEvent::Started { id });

            let mut child_status = None;
            let mut reader_done = false;
            let poll_interval = Duration::from_millis(10);
            
            let mut captured_output = Vec::new();
            
            let exit_status = loop {
                // The child_status being none means the child is still running, so we check it's status
                // again (without blocking) to see if it has exited now
                if child_status.is_none() {
                    // .try_wait() is a non-blocking operation that checks
                    // if the child process has completed the execution 
                    // if it has, and the process has been cleaned up,
                    // we will get the Ok(Some(ExitStatus)) 
                    // 
                    // If it's still running, we will get Ok(None), and the check
                    // if let Some(status) = child_status will get None so our code will keep running
                    // 
                    // We will get error if an error occured while quering the system for the status,
                    // which will return an Error
                    match child.try_wait() {
                        // We have gotten a Ok() meaning either the child process has finished (Ok(Some(T))) or is
                        // still running (Ok(None))
                        // 
                        // We put that Some() or None from inside Ok() to child_status variable which the code later
                        // on can use to determine whether we should continue or this loop should be exited 
                        Ok(status) => child_status = status,
                        Err(status_error) => {
                            // We try our best to clean things up before returning the original error we got
                            // from try_wait()
                            // 
                            // We intentionally ignore errors here because it's just our last effort in cleaning
                            // things up, we can't/don't need to do anything with the values returned by these
                            let _ = child.kill();
                            let _ = child.wait();
                            let _ = reader_thread.join();

                            // We must return here, cuz the join() consumes the reader_thread here, which moves here
                            // and can't be used later
                            // 
                            // Also if our try_wait() fails, there's nothing left for us to do, so we do our best to 
                            // not keep stale processes alive and finish the read buffer if it exists,
                            // This is our best attempt in doing so
                            return Err(RequestError::StatusError(status_error).into());
                        }
                    }
                }

                // See if we have the child_status, which will be Some() only when the 
                // child has finished the execution and the reader is done as well
                // which happens when the receiver has Disconnected
                if let (Some(status), true) = (child_status, reader_done) {
                    // we return the status so the code executing after the loop
                    // can use that status, eg; to return it in ExecutionResult
                    break status;
                }

                // If the child hasn't already exited (aka status none),
                // and the timeout isn't disabled,
                // 
                // See if the current elapsed timeout has matched/exceeded 
                // our timeout threshold
                if child_status.is_none() && !self.policy.disable_timeout() {                
                    let elapsed_time = start_time.elapsed();
                    let timeout = self.policy.timeout_s();

                    // Timeout has matched/exceeded our timeout threshold
                    if elapsed_time >= timeout {
                        // Kill the child process cuz it has matched/exceeded our timeout threshold
                        // 
                        // This terminates the process and moves it into a zombie state until the parent 
                        // that launched it; acknowledges its death
                        child.kill()
                            .map_err(RequestError::Kill)?;

                        // Reap the killed child and give all its resources back to the OS, this removes
                        // it from the zombie state and wipes it out
                        // 
                        // We dont use its exit status cuz 
                        // if we have reached here, then it means we have to return a
                        // Timeout error, not proceed forward
                        child.wait()
                            .map_err(RequestError::StatusError)?;
                        
                        // .join() returns immediately if the thread has already finished
                        // But we dont need it's result here cuz we already have got it,
                        // so we discard it
                        // 
                        // And if it has not, this will fully wait for the thread to finish,
                        reader_thread
                        .join()
                        .map_err(|_| RequestError::Join)?;

                        // Timed out, so return the error
                        return Err(RequestError::Timeout {
                            id,
                            elapsed_time,
                            timeout,
                            working_dir: self.policy.working_dir().to_path_buf()
                        }.into())
                    }
                }

                // Reader is done (means it has disconnected, see lhe Disconnected block)
                // meaning we have read the bytes, so sleep for poll_interval time and 
                // jump back to the top so it can get the status () and exit the loop
                // 
                // This is the signal that we can exit now, we dont try to read again 
                // after it
                if reader_done {
                    std::thread::sleep(poll_interval);
                    continue;
                }

                // Check for bytes every poll_interval time
                match rx.recv_timeout(poll_interval) {
                    Ok(Ok(bytes)) => {
                        // Put the bytes into the captured_output so we can return it together
                        // at last
                        captured_output.extend_from_slice(&bytes);

                        // Since we are streaming, give the bytes to the caller so they can use
                        // it immediately
                        on_event(ExecutionEvent::Output { id, bytes });
                    },
                    Ok(Err(error)) => {   
                        // Error occured, so terminate/kill the child process
                        child.kill()
                            .map_err(|e| RequestError::Kill(e))?;

                        // Let the child finish and get the exit status
                        child.wait()
                            .map_err(RequestError::StatusError)?;
                        
                        // Let the thread finish before we return the error (Ok() value discarded)
                        reader_thread
                            .join()
                            .map_err(|_| RequestError::Join)?;
                        
                        return Err(RequestError::MergedRead(error).into())
                    },

                    // We reached the timeout for this poll, so continue again
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        continue
                    },
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        // The reader is finished but don't break yet
                        // as the child may still be running.

                        // So mark the reader as done and let the code above to handle the
                        // status and byte reading
                        reader_done = true;
                    }
                }
            };

            // Make sure the thread has finished
            reader_thread
                .join()
                .map_err(|_| RequestError::Join)?;

            // Streaming is done, so give the final report
            Ok(
                ExecutionResult {
                    id, 
                    success: exit_status.success(),
                    status: exit_status.code(),
                    output: ExecutionOutput::Merged { 
                        output: captured_output,
                    },
                    working_dir: self.policy.working_dir().to_path_buf()
                }
            )
        }
    
    /// Normal Executer:
    /// 
    /// It takes a reference to the execution request, so you
    /// can execute the same request as many times as you want
    /// 
    /// Note that this returns the Result only when the process
    /// has exited, so it's not ideal to use if you want to get
    /// the output stream as it comes aka unideal for output streaming
    pub fn execute(&self) -> Result<ExecutionResult> {

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

        // tx = sender/transmitter
        // rx = receiver
        // let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>();
        // 
        // Create a channel for OutputMessage where we will pass the OutputMessage
        // which will allow us to distinguish whether the data is being sent by stdout or stderr
        // and whether they have returned Ok() or Err()
        let (tx, rx) = mpsc::channel::<OutputMessage>();

        // Stdout/stderr have separate senders but a single reader/receiver
        let tx_stdout = tx.clone();
        let tx_stderr = tx;

        // println!("Just created a channel for Vec<u8>");

        // Spawn a thread to read data from child's stdout pipe then send it to the channel
        // 
        // We make a thread for each because putting them into a single thread will
        // block the other while one's bytes are being read 
        std::thread::spawn(move || {
            let mut buffer = [0u8; 1024];

            loop {
                match stdout_stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        if tx_stdout.send(
                                OutputMessage::Stdout(
                                    Ok(buffer[..bytes_read].to_vec())
                            )
                        ).is_err() {
                            break;
                        }
                    },
                    Err(err) => {
                        // If error is occured, send the error and then break instead of 
                        // silently breaking
                        let _ = tx_stdout.send(
                            OutputMessage::Stdout(
                                Err(err)
                            )
                        );
                        break;
                    }
                }
            }
        });

        // Spawn a thread to read data from child's stderr pipe then send it to the channel
        std::thread::spawn(move || {
            let mut buffer = [0u8; 1024];

            loop {
                match stderr_stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        if tx_stderr.send(
                                OutputMessage::Stderr(
                                    Ok(buffer[..bytes_read].to_vec())
                            )
                        ).is_err() {
                            break;
                        }
                    },
                    Err(err) => {
                        let _ = tx_stderr.send(
                            OutputMessage::Stderr(
                                Err(err)
                            )
                        );
                        break;
                    }
                }
            }
        });
        
        let mut captured_stdout: Vec<u8> = Vec::new();
        let mut captured_stderr: Vec<u8> = Vec::new();
        
        // keep checking the incoming (non blocking) bytes in loop
        loop {
            let elapsed_time = start_time.elapsed();
            // if the process has ran for already over the timeout,
            // kill it before you do anything else
            // println!("About to check disable_timeout: {} and timeout_s: {:?}", disable_timeout, timeout_s);
            if (disable_timeout == false) && elapsed_time >= timeout_s {
                child.kill()
                    .map_err(|e| RequestError::Kill(e))?;

                // Propagate the error message instead of printing to terminal
                // 
                // But that would return too early and wont let us prepare the crash report later,
                // So prepare a rich status report here and return Err()
                return Err(
                    RequestError::Timeout {
                    id,
                    elapsed_time,
                    timeout: timeout_s,
                    working_dir: self.policy.working_dir().to_path_buf()
                }.into());
                // break;
            }

            match rx.try_recv() {
                // If the senders/transmitters has sent us something successfully, we get that inside Ok()
                // 
                // We look inside Ok() to check whether the OutputMessage we have gotten is from the Stdout sender or
                // Stderr sender
                // 
                // We see even deeper to check whether the respective sender has sent us an Err() or Ok()
                // 
                // We take appropriate action based on that
                Ok(OutputMessage::Stdout(Ok(stdout_bytes))) => captured_stdout.extend(stdout_bytes),
                Ok(OutputMessage::Stdout(Err(stdout_error))) => return Err(RequestError::StdoutRead(stdout_error).into()),
                
                Ok(OutputMessage::Stderr(Ok(stderr_bytes))) => captured_stderr.extend(stderr_bytes),
                Ok(OutputMessage::Stderr(Err(stderr_error))) => return Err(RequestError::StderrRead(stderr_error).into()),
                    
                Err(mpsc::TryRecvError::Empty) => {
                    // No bytes to read
                    // So sleep for 100 micro s and check again
                    std::thread::sleep(Duration::from_micros(100))
                },
                Err(mpsc::TryRecvError::Disconnected) => {
                    // We just know that both the stdout/stderr reader threads have dropped their senders

                    // EOF maybe??
                    
                    break;
                }
            }
        }

        // Wait till the child finishes and get its exit status
        let exit_status = child.wait()
            .map_err(RequestError::StatusError)?;

        // Now we dont need this because we are reading both stdout/stderr concurrently
        //
        // // Read the stderr if there is any
        // let mut captured_stderr: Vec<u8> = Vec::new();
        // stderr_stream.read_to_end(&mut captured_stderr)
        //     .map_err(|e| RequestError::StderrRead(e))?;

        Ok(ExecutionResult {
            id,
            success: exit_status.success(),
            status: exit_status.code(),
            output: ExecutionOutput::Separate{
                stdout: captured_stdout,
                stderr: captured_stderr,
            },
            working_dir: self.policy.working_dir().to_path_buf()
        })
    }

    pub fn arg<S: Into<OsString>>(mut self, arg: S) -> Self {
        self.args.push(arg.into());
        self
    }

    /// This is used to pass multiple arguments, Vec<> in this case
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
