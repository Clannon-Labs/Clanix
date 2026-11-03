use std::io::Read;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::Effective;
use crate::RequestError;
use crate::Result;
use crate::{ExecutionOutput, ExecutionResult};

use super::ExecutionRequest;
use super::OutputMessage;

/// Internal executor methods/helpers for blocking executions
impl ExecutionRequest<'_> {
    /// Normal Executer:
    ///
    /// It takes a reference to the execution request, so you
    /// can execute the same request as many times as you want
    ///
    /// Note that this returns the Result only when the process
    /// has exited, so it's not ideal to use if you want to get
    /// the output stream as it comes aka unideal for output streaming
    pub(super) fn execute_sync(&self, resolved: &Effective) -> Result<ExecutionResult> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .current_dir(resolved.working_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| RequestError::SpawnError {
                program: self.program.clone(),
                working_dir: resolved.working_dir().to_path_buf(),
                source,
            })?;

        let start_time = Instant::now();
        let id = Uuid::new_v4();

        // println!("Just set the start time and id");

        let mut stdout_stream = child
            .stdout
            .take()
            .ok_or(RequestError::StdoutUnavailable)?;

        let mut stderr_stream = child
            .stderr
            .take()
            .ok_or(RequestError::StderrUnavailable)?;

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
        let stdout_reader_thread = std::thread::spawn(move || {
            let mut buffer = [0u8; 1024];

            loop {
                match stdout_stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        if tx_stdout
                            .send(OutputMessage::Stdout(Ok(buffer[..bytes_read].to_vec())))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        // If error is occured, send the error and then break instead of
                        // silently breaking
                        let _ = tx_stdout.send(OutputMessage::Stdout(Err(err)));
                        break;
                    }
                }
            }
        });

        // Spawn a thread to read data from child's stderr pipe then send it to the channel
        let stderr_reader_thread = std::thread::spawn(move || {
            let mut buffer = [0u8; 1024];

            loop {
                match stderr_stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        if tx_stderr
                            .send(OutputMessage::Stderr(Ok(buffer[..bytes_read].to_vec())))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx_stderr.send(OutputMessage::Stderr(Err(err)));
                        break;
                    }
                }
            }
        });

        let mut captured_stdout: Vec<u8> = Vec::new();
        let mut captured_stderr: Vec<u8> = Vec::new();

        let mut child_status = None;
        let mut readers_done = false;

        // keep checking the incoming (non blocking) bytes in loop
        // to get the exit_status of the child, which will definetely be Some()
        // if the loop has exited
        let exit_status = loop {
            // If the child status is None, then it means when it was last checked
            // it hadn't exited,
            //
            // See if it has now and if it has, set the status which will cause the
            // child_status check later to break with the status (if reader is done as well)
            if child_status.is_none() {
                match child.try_wait() {
                    Ok(status) => child_status = status,
                    Err(status_error) => {
                        // Here, we just attempt a cleanup before returning the error because we
                        // dont want to stop the cleanup if either of the step fails cuz it's
                        // just our last cleanup attempt, not which must pass

                        // Kill the child process
                        let _ = child.kill();

                        // Take all the resources and erase child process completely
                        let _ = child.wait();

                        // Join the stdout/stderr handles and finish their readers
                        let _ = stdout_reader_thread.join();
                        let _ = stderr_reader_thread.join();

                        return Err(RequestError::StatusError(status_error).into());
                    }
                }
            }

            // Break with the status if we have the status and the reader is done
            if let (Some(status), true) = (child_status, readers_done) {
                break status;
            }

            if child_status.is_none() && !resolved.disable_timeout() {
                let elapsed_time = start_time.elapsed();
                let timeout = resolved.timeout();

                // if the process has ran for already over the timeout,
                // kill it before you do anything else
                if elapsed_time >= timeout {
                    // Attempt a last cleanup and discard their values because we already have our error we
                    // are trying to return
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader_thread.join();
                    let _ = stderr_reader_thread.join();

                    // Propagate the error message instead of printing to terminal
                    //
                    // But that would return too early and wont let us prepare the crash report later,
                    // So prepare a rich status report here and return Err()
                    return Err(RequestError::Timeout {
                        id,
                        elapsed_time,
                        timeout,
                        working_dir: resolved.working_dir().to_path_buf(),
                    }
                    .into());
                }
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
                Ok(OutputMessage::Stdout(Err(stdout_error))) => {
                    // Attempt a full cleanup before returning an error so we dont accidently leave
                    // any zombie/stale process after we exit from the function
                    //
                    // And since we can't do the cleanup after this function returns
                    // the stale processes will dangle throughout the clanix process
                    //
                    // We deliberately discard the values returned by the cleanup steps cuz we already
                    // have prepared the error we are trying to return, we dont want to return the errors
                    // of the cleanup attempts here
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader_thread.join();
                    let _ = stderr_reader_thread.join();

                    return Err(RequestError::StdoutRead(stdout_error).into());
                }

                Ok(OutputMessage::Stderr(Ok(stderr_bytes))) => captured_stderr.extend(stderr_bytes),
                Ok(OutputMessage::Stderr(Err(stderr_error))) => {
                    // Attempt a cleanup but discard the values
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader_thread.join();
                    let _ = stderr_reader_thread.join();

                    return Err(RequestError::StderrRead(stderr_error).into());
                }

                Err(mpsc::TryRecvError::Empty) => {
                    // No bytes to read
                    // So sleep for 100 micro s and check again
                    std::thread::sleep(Duration::from_micros(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // We just know that both the stdout/stderr reader threads have dropped their senders

                    // EOF maybe??
                    readers_done = true;
                    // break;
                }
            }
        };

        // Join the reader threads one last time to make sure they have completed the job
        //
        // And now that we are no more checking for anything, we must ensure all the process
        // has exited, and since we are preparing an Ok() return now, we must keep everything clean
        // so we error out if some processes can't be joined/completed
        stdout_reader_thread
            .join()
            .map_err(|_| RequestError::Join)?;

        stderr_reader_thread
            .join()
            .map_err(|_| RequestError::Join)?;

        Ok(ExecutionResult {
            id,
            success: exit_status.success(),
            status: exit_status.code(),
            output: ExecutionOutput::Separate {
                stdout: captured_stdout,
                stderr: captured_stderr,
            },
            working_dir: resolved.working_dir().to_path_buf(),
        })
    }
}
