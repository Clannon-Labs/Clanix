use std::ffi::OsString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Serve,
    Doctor,
    Version,
    Help,
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let arguments: Vec<_> = arguments.into_iter().collect();
    match arguments.as_slice() {
        [] => Ok(Command::Serve),
        [argument] if argument == "serve" => Ok(Command::Serve),
        [argument] if argument == "doctor" => Ok(Command::Doctor),
        [argument] if argument == "--version" || argument == "-V" => Ok(Command::Version),
        [argument] if argument == "--help" || argument == "-h" => Ok(Command::Help),
        [argument] => Err(format!("unknown command or option {:?}", argument)),
        _ => Err("Clannon accepts one command at a time".to_owned()),
    }
}

pub(crate) fn usage() -> &'static str {
    "Usage: clannon [serve|doctor|--version|--help]\n\nCommands:\n  serve       Start the local Clannon server (default)\n  doctor      Check Linux, the local listen address, and rootless Podman\n\nOptions:\n  --version   Print the Clannon version\n  --help      Print this help"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(arguments: &[&str]) -> Result<Command, String> {
        parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_and_serve_keep_normal_server_behavior() {
        assert_eq!(parse_str(&[]), Ok(Command::Serve));
        assert_eq!(parse_str(&["serve"]), Ok(Command::Serve));
    }

    #[test]
    fn parses_release_utility_commands() {
        assert_eq!(parse_str(&["doctor"]), Ok(Command::Doctor));
        assert_eq!(parse_str(&["--version"]), Ok(Command::Version));
        assert_eq!(parse_str(&["-V"]), Ok(Command::Version));
        assert_eq!(parse_str(&["--help"]), Ok(Command::Help));
        assert_eq!(parse_str(&["-h"]), Ok(Command::Help));
    }

    #[test]
    fn rejects_unknown_and_combined_commands() {
        assert!(parse_str(&["unknown"]).unwrap_err().contains("unknown"));
        assert_eq!(
            parse_str(&["doctor", "--version"]),
            Err("Clannon accepts one command at a time".into())
        );
    }
}
