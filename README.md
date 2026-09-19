# Clanix

First commit of Clanix - [Cybro](https://github.com/thecybro)

## Examples of supported type of commands (basic for now): 
> Note that the initial `cargo run --` will be replaced by `clanix` when running from the binary

> (basic)
- `cargo run -- -s output.txt run cargo new project`
- `cargo run -- -v test`
> (chained)
- `cargo run -- -v -s output3.txt run cargo new project2`
> (output format, not passing -f format would output a json)
- `cargo run -- -s test.txt -f text run cargo new prj`
> (to write content to file)
- `cargo run -- -s test.txt -v -f json write "Text from Clanix" test1.txt`
> (clear)
- `cargo run run clear`
- 