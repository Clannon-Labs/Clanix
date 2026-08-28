use std::net::SocketAddr;

use runtime::Runtime;

mod access;
mod app;
mod cli;
mod doctor;
mod error;
mod terminal;

const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_IMAGE: &str = "docker.io/library/alpine@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b";

#[tokio::main]
async fn main() {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}\n\n{}", cli::usage());
            std::process::exit(2);
        }
    };
    let exit_code = match command {
        cli::Command::Serve => serve().await,
        cli::Command::Doctor => {
            let bind = std::env::var("CLANNON_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
            let report = doctor::run(&bind).await;
            println!("{report}");
            i32::from(!report.passed())
        }
        cli::Command::Version => {
            println!("clannon {}", env!("CARGO_PKG_VERSION"));
            0
        }
        cli::Command::Help => {
            println!("{}", cli::usage());
            0
        }
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

async fn serve() -> i32 {
    let bind = std::env::var("CLANNON_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let image = std::env::var("CLANNON_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_owned());
    let address = match parse_bind(&bind) {
        Ok(address) => address,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };

    if let Err(error) = Runtime::verify_rootless().await {
        eprintln!("Clannon requires a working rootless Podman installation: {error}");
        return 1;
    }

    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("could not listen on {address}: {error}");
            return 1;
        }
    };
    let local_address = match listener.local_addr() {
        Ok(address) => address,
        Err(error) => {
            eprintln!("could not determine listening address: {error}");
            return 1;
        }
    };
    let access = match access::AccessPolicy::generate(local_address) {
        Ok(access) => access,
        Err(error) => {
            eprintln!("could not generate the local access capability: {error}");
            return 1;
        }
    };
    let runtime = Runtime::new(image);
    let state = app::AppState::new(runtime.clone(), access.clone());
    let router = app::routes(state);

    println!("Clannon is ready at {}", access.startup_url());
    let server_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_and_cleanup(runtime.clone()))
        .await;
    runtime.cleanup().await;
    if let Err(error) = server_result {
        eprintln!("server error: {error}");
        return 1;
    }
    0
}

fn parse_bind(bind: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = bind
        .parse()
        .map_err(|error| format!("invalid CLANNON_BIND {bind:?}: {error}"))?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "unsupported CLANNON_BIND {bind:?}: Clannon only accepts loopback addresses"
        ));
    }
    Ok(address)
}

async fn shutdown_and_cleanup(state: Runtime) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    state.cleanup().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_accepts_loopback_and_preserves_port_zero() {
        assert_eq!(
            parse_bind("127.0.0.1:0").unwrap(),
            "127.0.0.1:0".parse().unwrap()
        );
        assert_eq!(parse_bind("[::1]:0").unwrap(), "[::1]:0".parse().unwrap());
    }

    #[test]
    fn bind_rejects_non_loopback_addresses() {
        let error = parse_bind("0.0.0.0:3000").unwrap_err();
        assert!(error.contains("only accepts loopback addresses"));
    }

    #[test]
    fn default_guest_image_is_immutable() {
        assert!(DEFAULT_IMAGE.starts_with("docker.io/library/alpine@sha256:"));
        assert_eq!(DEFAULT_IMAGE.len(), 96);
    }
}
