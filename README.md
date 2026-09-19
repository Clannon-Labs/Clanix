# Clanix

First commit of Clanix - [Cybro](https://github.com/thecybro)

## Examples of supported type of commands (basic for now):
> [!NOTE]
> The commands below are just to test *Clanix* running commands,
> not what *Clanix* is made to do, after the very first release, *Clanix* will
> do much more, not merely run some basic commands.

> If you dont have the binary yet,
> replace `clanix` with `cargo run` or `cargo run --` if using `-` or `--` in the command 

> (basic)
- `clanix -s output.txt run cargo new project`
- `clanix -v test`
> (chained)
- `clanix -v -s output3.txt run cargo new project2`
> (output format, not passing -f format would output a json)
- `clanix -s test.txt -f text run cargo new prj`
> (to write content to file)
- `clanix -s test.txt -v -f json write "Text from Clanix" test1.txt`
> (clear)
- `clanix run clear`
- 
