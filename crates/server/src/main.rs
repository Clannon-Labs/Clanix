use std::net::SocketAddr;

use runtime::Runtime;

mod access;
mod app;
mod error;
mod terminal;

const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_IMAGE: &str = "docker.io/library/alpine:3.20";

#[tokio::main]
async fn main() {
    let bind = std::env::var("CLANNON_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let image = std::env::var("CLANNON_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_owned());
    let address = parse_bind(&bind).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });

    if let Err(error) = Runtime::verify_rootless().await {
        eprintln!("Clannon requires a working rootless Podman installation: {error}");
        std::process::exit(1);
    }

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .unwrap_or_else(|error| {
            eprintln!("could not listen on {address}: {error}");
            std::process::exit(1);
        });
    let local_address = listener.local_addr().unwrap_or_else(|error| {
        eprintln!("could not determine listening address: {error}");
        std::process::exit(1);
    });
    let access = access::AccessPolicy::generate(local_address).unwrap_or_else(|error| {
        eprintln!("could not generate the local access capability: {error}");
        std::process::exit(1);
    });
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
    }
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
}
