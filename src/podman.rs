use std::{io, process::Stdio};

use axum::http::StatusCode;
use tokio::process::{Child, Command};

use crate::error::AppError;

pub(crate) async fn verify_rootless() -> Result<(), String> {
    let output = Command::new("podman")
        .args(["info", "--format", "{{.Host.Security.Rootless}}"])
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    if String::from_utf8_lossy(&output.stdout).trim() != "true" {
        return Err("Podman is not running rootless".to_owned());
    }
    Ok(())
}

pub(crate) async fn create_container(name: &str, image: &str) -> Result<(), AppError> {
    let output = Command::new("podman")
        .args([
            "run",
            "--detach",
            "--rm",
            "--name",
            name,
            "--cap-drop=all",
            "--security-opt=no-new-privileges",
            "--pids-limit=256",
            "--memory=512m",
            "--cpus=1",
            "--tmpfs",
            "/workspace:rw,exec,nosuid,size=256m",
            "--workdir=/workspace",
            image,
            "/bin/sh",
            "-c",
            "trap 'exit 0' TERM INT; while :; do sleep 3600; done",
        ])
        .output()
        .await
        .map_err(|error| AppError::internal("could not start Podman", error))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            "could not create environment",
            &output.stderr,
        ))
    }
}

pub(crate) fn spawn_terminal(container: &str) -> io::Result<Child> {
    Command::new("podman")
        .args([
            "exec",
            "-i",
            "--workdir",
            "/workspace",
            container,
            "/bin/sh",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

pub(crate) async fn exec(container: &str, script: &str) -> Result<String, AppError> {
    let output = Command::new("podman")
        .args(["exec", container, "/bin/sh", "-c", script])
        .output()
        .await
        .map_err(|error| AppError::internal("could not inspect environment", error))?;
    if !output.status.success() {
        return Err(command_error(
            "environment inspection failed",
            &output.stderr,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) async fn remove_container(name: &str) -> Result<(), AppError> {
    let output = Command::new("podman")
        .args(["rm", "--force", "--ignore", name])
        .output()
        .await
        .map_err(|error| AppError::internal("could not destroy environment", error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            "could not destroy environment",
            &output.stderr,
        ))
    }
}

fn command_error(context: &str, stderr: &[u8]) -> AppError {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    AppError::new(
        StatusCode::BAD_GATEWAY,
        if detail.is_empty() {
            context.to_owned()
        } else {
            format!("{context}: {detail}")
        },
    )
}
