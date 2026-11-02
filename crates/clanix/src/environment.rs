
use std::ffi::OsString;
use std::cell::{Ref, RefMut, RefCell};

use serde::{Deserialize, Serialize};

use crate::{ExecutionRecord, ExecutionEvent, ExecutionRequest, ExecutionResult};
use crate::Policy;
use crate::Config;
use crate::Resolver;
use crate::{Error, Result};

/// The main Environment struct that provides a clean set of APIs the users can
/// call to manage the environment more easily instead of having to think about every
/// minor detail
/// 
/// But note that you can still call the methods provided by each layer (i.e. ExecutionRequest,
/// ExecutionResult, Config etc); only if you want finer controls on the environment ofc
/// 
/// If you just want to use the environment without having to coordinate between multiple stages
/// and managing each thing, the Environment will increasingly provide a better set of APIs
/// to do so
/// 
#[derive(Debug, Serialize, Deserialize)]
pub struct Environment {
    pub(crate) history: RefCell<Vec<ExecutionRecord>>,
    resolver: Resolver
}

impl Environment {
    /// Create a brand new environment instance with the default values
    pub fn new() -> Self {
        Self { 
            history: RefCell::default(),
            resolver: Resolver::default(),
        }
    }

    pub fn with_config(config: Config) -> Self {
        Self { 
            history: RefCell::default(),
            resolver: Resolver::new(config)
        }
    }

    pub fn from_disk() -> Result<Self> {
        Ok(Self { 
            history: RefCell::default(),
            resolver: Resolver::from_disk()
               .map_err(Error::Resolve)? 
        })
    }
    
    /// Create a new execution request instance.
    /// 
    /// Note that this just creates an instance of execution request,
    /// It doesn't actualy execute the request
    /// 
    /// It lets you get the ExecutionRequest instance the moment you call it
    /// and pass the arguments to it along the way (i.e. by chaining commands)
    /// and at last, when you want to actually execute it, you can call the
    /// executer method
    pub fn new_request<S: Into<OsString>>(&self, program: S) -> ExecutionRequest<'_> {
        ExecutionRequest {
            env: Some(self),
            program: program.into(),
            args: Vec::new(),
            policy: Policy::default(),
        }
    }

    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }

    pub fn history(&self) -> Ref<'_, Vec<ExecutionRecord>> {
        self.history.borrow()
    }

    pub fn history_mut(&self) -> RefMut<'_, Vec<ExecutionRecord>> {
        self.history.borrow_mut()
    }
    
    pub fn attach<'a>(&'a self, request: ExecutionRequest<'static>) -> ExecutionRequest<'a> {
        ExecutionRequest {
            env: Some(self),
            program: request.program,
            args: request.args,
            policy: request.policy
        }
    }
    
    /// If you created the request inside the Environment, your request will automatically
    /// have a reference to the environment; so you dont have to worry about strictly
    /// executing request via the environment only; 
    /// 
    /// For example;
    /// ```no_run
    ///     let env = Environment::new();
    ///     let request = env.new_request("echo");
    /// 
    ///     // In any of the setters in the request, you can give it any of the 
    ///     // T and Some(T) and it will automatically use the T if available;
    ///     // if not, it will just return the request unmutated
    ///     env.arg("Hey");  
    ///     // Or env.arg(Some("Hey"))
    ///     
    ///     // While it is techically valid for all setters, giving None inplace of 
    ///     // args might produce unexpected results
    ///     // Or env.arg(None) 
    /// 
    ///     let result = env.execute_request(&request)?;
    ///     // Or request.execute()?; 
    ///     // Here, both are identical and wont do anything different
    ///                 
    /// ```
    /// 
    pub fn execute_request(
        &self,
        request: &ExecutionRequest<'_>
    ) -> Result<ExecutionResult> {
        
        request.execute()
    }

    /// This method is also the same as the above,
    /// But for non-blocking/streaming requests where
    pub fn execute_request_with_events<F>(
        &self, 
        request: &ExecutionRequest,
        on_event: F
    ) -> Result<ExecutionResult> 
        where 
            F: FnMut(ExecutionEvent)
    {
        request.execute_with_events(on_event)
    }

    /// Set the config to default
    pub fn set_default_config(&mut self) -> Result<Config> {
        let config = Config::set_default()?;
        self.resolver = Resolver::new(config);
        Ok(Config::default())
        
    }

    pub fn last_execution(&self) -> Option<Ref<'_, ExecutionRecord>> {
        Ref::filter_map(self.history(), |his| his.last()).ok()
    }

    pub fn last_result(&self) -> Option<Ref<'_, ExecutionResult>> {
        Ref::filter_map(self.history(), |his| his.last())
            .map(|record| Ref::map(record, |r| r.result()))
            .ok()
    }

    pub fn last_result_mut(&self) -> Option<RefMut<'_, ExecutionResult>> {
        RefMut::filter_map(self.history_mut(), |his| his.last_mut())
            .map(|record| RefMut::map(record, |r| &mut r.result))
            .ok()
    }
    
    /// Get the reference of config instance
    pub fn config(&self) -> &Config {
        &self.resolver.config()
    }

    /// Get the mutable reference of config instance
    pub fn config_mut(&mut self) -> &mut Config {
        self.resolver.config_mut()
    }

}

impl Environment {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Error::from)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(Error::from)
    }
}


impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\nENVIRONMENT")?;
        writeln!(f, "=============")?;

        let history = self.history.borrow();
        if !history.is_empty() {
            for record in history.iter() {
                writeln!(f, "{:#?}", record)?;
            }
        }

        writeln!(f, "{:#?}", self.resolver)?;
        
        Ok(())
    }
}
