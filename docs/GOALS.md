> What I am thinking of building towards for Clanix.

1. Project discovery:
> Just started; not done yet

  To make `Clanix` able to detect what type of project is the user right now
  Clanix could look for directories or files, like:
  cargo.toml: rust project
  package.json: Javascript/Typescript project
  .venv/pyproject.toml: python project
  .git/: git repository etc

2. A proper Clanix project manifest:
> Planning to start it now

 Clanix having a configuration file (maybe clanix.toml?) which will
 define the project identity and version
 supported languages, build/test commands, environment variables etc
 just like cargo.toml is for cargo.

> [!NOTE]
> Current `clanix.toml` is a basic config file which defines very basic things only
> maybe we could separate it or keep those policy related configs as a section of
> `clanix.toml` instead of being the whole file of their own
  
Like this maybe?
```toml
[project]
name = "my-app"

[tools]
rust = true
python = true

[tasks]
test = "cargo test"
```

3. Artifact Tracking:

  Making the execution result richer so that it can show us the following instead of
  just success/fail

  For example, it might also collect or report:
  - Generated files
  - Their paths
  - Sizes and modification times
  - What task produced them
  - and so on

4. Execution history and diagnostics

  It could allow you to:
  - See the history of runs
  - Inspect any runs
  - See difference between any runs
  - See the environment and result of any run

5. Tool adapters and capability detection

  Making a separate adapter for each language/type of project
  Like if my detector detected a rust project, my rust detector
  should do the later job, and if my detector detected python,
  python adapter should do the work

  Keep the adapters different for each type of project/language

