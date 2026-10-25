I have an idea for the config struct, I think separating the concerns with nested structs is better

So instead of having 
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    disable_timeout: bool,
    timeout_s: Duration,
    working_dir: PathBuf,
}
```

We could have something like 
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    workspace: Workspace
    time: TimeConfig
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    working_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeConfig {
    disable_timeout: bool,
    timeout_s: Duration,
}
```

And instead of writing all the fields in the config, we could make the fields optional
if that's not required to run Clanix

This could allow users to write the config of the fields they want and keep whatever
else fields we have (could even be in the 100s if that's needed), so users can tweak it
as much as they want but they dont have to, I got this idea from cargo, where i can write
the config if I want to, if not, it will just use the default or not just use it
