use std::io::Read;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::{RequestError, Result};

use crate::{Effective, ExecutionEvent, ExecutionOutput, ExecutionResult};

use super::ExecutionRequest;

/// Internal executer methods/helpers for non-blocking/streaming executions
impl<'env> ExecutionRequest<'env> {
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
    pub(super) fn execute_callback<F>(
        &self,
        resolved: &Effective,
        mut on_event: F,
    ) -> Result<ExecutionResult>
    where
        F: FnMut(ExecutionEvent),
    {
        // Ask the OS for a connected reader/writer pair where the
        // bytes written in stdout_writer can be read from the reader
        let (reader, stdout_writer) = os_pipe::pipe().map_err(RequestError::MergedPipe)?;

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
            .current_dir(resolved.working_dir())
            .stdout(stdout_writer)
            .stderr(stderr_writer)
            .spawn()
            .map_err(|source| RequestError::SpawnError {
                program: self.program.clone(),
                working_dir: resolved.working_dir().to_path_buf(),
                source,
            })?;

        let start_time = Instant::now();
        let id = Uuid::new_v4();

        // Channel to transmit & receive the bytes
        let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>();

        // Spawn a thread where the reader can keep reading bytes and send the
        // bytes over to our main thread over the channel (which has rx as the receiver)
        //
        // This keeps giving reader the messages through the channel which it can use
        // to inspect, block, kill, join the reader thread etc

        // spawn() starts the reader on another thread and returns a JoinHandle
        // and the reader can block on pipe reads without blocking the main thread
        //
        // The handle it gets can later be joined to wait for completion and detect panics
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
                    //
                    // It definitely means EOF and the pipe has closed, and that's the reason
                    // no bytes were remaining; hence we getting 0 of them
                    //
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
                        if tx.send(Ok(buffer[..bytes_read].to_vec())).is_err() {
                            break;
                        }
                    }
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
            if child_status.is_none() && !resolved.disable_timeout() {
                let elapsed_time = start_time.elapsed();
                let timeout = resolved.timeout();

                // Timeout has matched/exceeded our timeout threshold
                if elapsed_time >= timeout {
                    // Kill the child process cuz it has matched/exceeded our timeout threshold
                    //
                    // This terminates the process and moves it into a zombie state until the parent
                    // that launched it; acknowledges its death
                    let _ = child.kill();

                    // Reap the killed child and give all its resources back to the OS, this removes
                    // it from the zombie state and wipes it out
                    //
                    // We dont use its exit status cuz
                    // if we have reached here, then it means we have to return a
                    // Timeout error, not proceed forward
                    let _ = child.wait();

                    // .join() returns immediately if the thread has already finished
                    // But we dont need it's result here cuz we already have got it,
                    // so we discard it
                    //
                    // And if it has not, this will fully wait for the thread to finish,
                    let _ = reader_thread.join();

                    // Timed out, so return the error
                    return Err(RequestError::Timeout {
                        id,
                        elapsed_time,
                        timeout: resolved.timeout(),
                        working_dir: resolved.working_dir().to_path_buf(),
                    }
                    .into());
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
                }
                Ok(Err(error)) => {
                    // Since we already have the error prepared, we just attempt a last cleanup

                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader_thread.join();

                    return Err(RequestError::MergedRead(error).into());
                }

                // We reached the timeout for this poll, so continue again
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
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
        reader_thread.join().map_err(|_| RequestError::Join)?;

        // Streaming is done, so give the final report
        Ok(ExecutionResult {
            id,
            success: exit_status.success(),
            status: exit_status.code(),
            output: ExecutionOutput::Merged {
                output: captured_output,
            },
            working_dir: resolved.working_dir().to_path_buf(),
        })
    }
}
