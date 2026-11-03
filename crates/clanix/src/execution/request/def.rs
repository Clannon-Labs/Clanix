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

use std::borrow::Cow;
use std::time::{SystemTime, Duration};
use std::ffi::OsString;
use std::path::PathBuf;


use serde::{Serialize, Deserialize, Serializer, Deserializer};

use crate::{
    Result,
    RequestError, 

    Environment,
    Policy,
    Resolver,
    Effective,

    ExecutionResult,
    ExecutionEvent,
    ExecutionRecord
};

#[derive(Debug)]
pub enum OutputMessage {
    Stdout(std::io::Result<Vec<u8>>),
    Stderr(std::io::Result<Vec<u8>>),
}

#[must_use]
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest<'env> {
    #[serde(skip)]
    pub(crate) env: Option<&'env Environment>,
    
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

fn deserialize_os_string<'de, D>(
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
        S: Serializer
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
        D: Deserializer<'de>
    {
        Vec::<String>::deserialize(deserializer)
            .map(|values| values.into_iter()
                .map(OsString::from)
                .collect())
    }


impl std::fmt::Display for ExecutionRequest<'_> {
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

/// These method(s) work just for standalone ExecutionRequest (meaning no environment)
impl ExecutionRequest<'static> {
    /// Method to create a new instance of ExecutionRequest regardless of environment
    /// 
    /// Currently, this is the only standalone method on the ExecutoinRequest because
    /// it creates a request with no environment
    pub fn new<S: Into<OsString>>(program: S) -> Self {
        Self {
            env: None,
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default()
        }
    }
}

/// These methods work for both standalone and environment bound instances
impl<'env> ExecutionRequest<'env> {
    fn prepare(&self, resolver: &Resolver) -> Result<Effective> {
        let effective = resolver.resolve(&self.policy)?;
        
        Ok(effective)
    }

    fn env_resolver(&self) -> Cow<'_, Resolver> {
        match self.env {
            Some(env) => Cow::Borrowed(env.resolver()),
            None => Cow::Owned(Resolver::default()),
        }
    }

    fn snapshot(&self) -> ExecutionRequest<'static> {
        ExecutionRequest {
            env: None, // detach the environment
            program: self.program.clone(),
            args: self.args.clone(),
            policy: self.policy.clone()
        }
    }
    
    /// Public api for blocking executions
    pub fn execute(&self) -> Result<ExecutionResult> {
        let effective = self.prepare(&self.env_resolver())?;
        
        let (started_at, result) = (
            SystemTime::now(),
            self.execute_sync(&effective)?
        );

        // Record the end to end workflow of the run
        ExecutionRecord::record(self.snapshot(), effective, started_at, &result);

        Ok(result)
    }

    /// Public api for non-blocking/streaming executions
    pub fn execute_with_events<F>(
        &self,
        on_event: F
    ) -> Result<ExecutionResult>
        where 
            F: FnMut(ExecutionEvent)
        {
            let effective = self.prepare(&self.env_resolver())?;
            
            let (started_at, result) = (
                SystemTime::now(),
                self.execute_callback(&effective, on_event)?
            );

            // Finished streaming; now record it
            ExecutionRecord::record(self.snapshot(), effective, started_at, &result);

            Ok(result)
        }
}


impl<'env> ExecutionRequest<'env> {
    /// Used to pass a single argument
    /// You can pass both Option<T> and T
    pub fn arg<S, T>(&mut self, arg: T) -> &mut Self 
        where 
            T: Into<Option<S>>,
            S: Into<OsString>
        {
            if let Some(a) = arg.into() {
                self.args.push(a.into());
            }
            self
        }
    
    /// This is used to pass multiple arguments stored in any kind of iterator
    /// Pass either T or Option<T>
    pub fn args<I, S, T>(&mut self, args: T) -> &mut Self
        where
            T: Into<Option<I>>,
            I: IntoIterator<Item = S>,
            S: Into<OsString>,
        {
            if let Some(arguments) = args.into() {
                for arg in arguments {
                    self.args.push(arg.into());
                }
            }
            self
        }
}

/// Getters/setters for ExecutionRequest
impl<'env> ExecutionRequest<'env> {
    pub fn policy(&self) -> &Policy {
        &self.policy
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
    pub fn timeout<T>(&mut self, timeout: T) -> &mut Self 
        where 
            T: Into<Option<Duration>>,
        {
            if let Some(time) = timeout.into() {
                self.policy.set_timeout(time);
            }
            self
        }

    pub fn disable_timeout(&mut self, disable: bool) -> &mut Self {
        self.policy.set_disable_timeout(disable);
        self
    }

    pub fn working_dir<T>(&mut self, working_dir: T) -> &mut Self
        where 
            T: Into<Option<PathBuf>>,
        {
            if let Some(dir) = working_dir.into() {
                self.policy.set_working_dir(dir.into());
            }
            self
        }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self)
            .map_err(|e| RequestError::JsonError(e).into())
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(&self)
            .map_err(|e| RequestError::TomlError(e).into())
    }
}

