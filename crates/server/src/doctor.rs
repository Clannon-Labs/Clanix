use std::{fmt, net::TcpListener};

use runtime::Runtime;

use crate::parse_bind;

pub(crate) struct Report {
    checks: Vec<Check>,
}

struct Check {
    name: &'static str,
    result: Result<(), String>,
}

impl Report {
    fn new(checks: Vec<Check>) -> Self {
        Self { checks }
    }

    pub(crate) fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.result.is_ok())
    }

    fn problem_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.result.is_err())
            .count()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for check in &self.checks {
            match &check.result {
                Ok(()) => writeln!(formatter, "[ok]   {}", check.name)?,
                Err(error) => writeln!(formatter, "[fail] {}: {error}", check.name)?,
            }
        }
        if self.passed() {
            write!(formatter, "Clannon doctor passed.")
        } else {
            write!(
                formatter,
                "Clannon doctor found {} problem(s).",
                self.problem_count()
            )
        }
    }
}

pub(crate) async fn run(bind: &str) -> Report {
    let linux = check_linux(std::env::consts::OS);
    let listen = check_listen_address(bind);
    let podman = Runtime::verify_rootless().await;
    Report::new(vec![
        Check {
            name: "Linux host",
            result: linux,
        },
        Check {
            name: "local listen address",
            result: listen,
        },
        Check {
            name: "rootless Podman",
            result: podman,
        },
    ])
}

fn check_linux(os: &str) -> Result<(), String> {
    if os == "linux" {
        Ok(())
    } else {
        Err(format!(
            "this local alpha supports Linux, but the current host is {os}"
        ))
    }
}

fn check_listen_address(bind: &str) -> Result<(), String> {
    let address = parse_bind(bind)?;
    let listener = TcpListener::bind(address)
        .map_err(|error| format!("could not bind configured address {address}: {error}"))?;
    drop(listener);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_and_ephemeral_loopback_are_valid() {
        assert_eq!(check_linux("linux"), Ok(()));
        assert_eq!(check_listen_address("127.0.0.1:0"), Ok(()));
    }

    #[test]
    fn unsupported_host_and_public_bind_are_actionable() {
        assert!(check_linux("macos").unwrap_err().contains("supports Linux"));
        assert!(
            check_listen_address("0.0.0.0:3000")
                .unwrap_err()
                .contains("only accepts loopback")
        );
    }

    #[test]
    fn report_aggregates_failures_and_has_a_stable_summary() {
        let report = Report::new(vec![
            Check {
                name: "first",
                result: Ok(()),
            },
            Check {
                name: "second",
                result: Err("missing".into()),
            },
        ]);

        assert!(!report.passed());
        assert_eq!(
            report.to_string(),
            "[ok]   first\n[fail] second: missing\nClannon doctor found 1 problem(s)."
        );
    }
}
