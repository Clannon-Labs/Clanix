use std::net::SocketAddr;

use runtime::Runtime;

mod app;
mod error;
mod terminal;

const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_IMAGE: &str = "docker.io/library/alpine:3.20";

#[tokio::main]
async fn main() {
    let bind = std::env::var("CLANNON_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let image = std::env::var("CLANNON_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_owned());
    let address: SocketAddr = bind.parse().unwrap_or_else(|error| {
        eprintln!("invalid CLANNON_BIND {bind:?}: {error}");
        std::process::exit(2);
    });

    if let Err(error) = Runtime::verify_rootless().await {
        eprintln!("Clannon requires a working rootless Podman installation: {error}");
        std::process::exit(1);
    }

    let state = Runtime::new(image);
    let router = app::routes(state.clone());
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .unwrap_or_else(|error| {
            eprintln!("could not listen on {address}: {error}");
            std::process::exit(1);
        });

    println!("Clannon is ready at http://{address}");
    let server_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_and_cleanup(state.clone()))
        .await;
    state.cleanup().await;
    if let Err(error) = server_result {
        eprintln!("server error: {error}");
    }
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
