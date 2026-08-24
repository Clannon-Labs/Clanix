use std::{net::Ipv6Addr, sync::Arc};

use serde::Serialize;

use crate::{
    environment::{Environment, TranscriptEntry, now_ms},
    podman,
};

#[derive(Serialize)]
pub(crate) struct ObservationSnapshot {
    environment_id: String,
    captured_at_ms: u128,
    transcript: Vec<TranscriptEntry>,
    processes: Vec<ProcessObservation>,
    files: Vec<FileObservation>,
    network: Vec<NetworkObservation>,
    warnings: Vec<String>,
}

#[derive(Debug, PartialEq, Serialize)]
struct ProcessObservation {
    pid: u32,
    parent_pid: u32,
    state: String,
    command: String,
    arguments: String,
}

#[derive(Debug, PartialEq, Serialize)]
struct FileObservation {
    path: String,
    size_bytes: u64,
    modified_unix_seconds: u64,
    kind: String,
}

#[derive(Debug, PartialEq, Serialize)]
struct NetworkObservation {
    protocol: String,
    local_address: String,
    remote_address: String,
    state: String,
}

pub(crate) async fn collect(environment: Arc<Environment>) -> ObservationSnapshot {
    let mut warnings = Vec::new();

    let processes = match podman::exec(
        environment.container_name(),
        "ps -o pid,ppid,stat,comm,args",
    )
    .await
    {
        Ok(raw) => parse_processes(&raw),
        Err(error) => {
            warnings.push(error.into_message());
            Vec::new()
        }
    };
    let files = match podman::exec(
        environment.container_name(),
        "find /workspace -mindepth 1 -maxdepth 4 -exec stat -c '%n|%s|%Y|%F' '{}' ';' 2>/dev/null | head -200",
    ).await {
        Ok(raw) => parse_files(&raw),
        Err(error) => { warnings.push(error.into_message()); Vec::new() }
    };
    let network = match podman::exec(
        environment.container_name(),
        "for f in tcp tcp6 udp udp6; do echo __${f}__; cat /proc/net/$f 2>/dev/null; done",
    )
    .await
    {
        Ok(raw) => parse_network(&raw),
        Err(error) => {
            warnings.push(error.into_message());
            Vec::new()
        }
    };
    let transcript = environment.transcript_snapshot().await;

    ObservationSnapshot {
        environment_id: environment.id().to_owned(),
        captured_at_ms: now_ms(),
        transcript,
        processes,
        files,
        network,
        warnings,
    }
}

fn parse_processes(raw: &str) -> Vec<ProcessObservation> {
    raw.lines()
        .skip_while(|line| line.trim_start().starts_with("PID"))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent_pid = fields.next()?.parse().ok()?;
            let state = fields.next()?.to_owned();
            let command = fields.next()?.to_owned();
            let arguments = fields.collect::<Vec<_>>().join(" ");
            Some(ProcessObservation {
                pid,
                parent_pid,
                state,
                command,
                arguments,
            })
        })
        .collect()
}

fn parse_files(raw: &str) -> Vec<FileObservation> {
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.rsplitn(4, '|');
            let kind = fields.next()?.to_owned();
            let modified_unix_seconds = fields.next()?.parse().ok()?;
            let size_bytes = fields.next()?.parse().ok()?;
            let path = fields.next()?.to_owned();
            Some(FileObservation {
                path,
                size_bytes,
                modified_unix_seconds,
                kind,
            })
        })
        .collect()
}

fn parse_network(raw: &str) -> Vec<NetworkObservation> {
    let mut protocol = String::new();
    let mut observations = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(marker) = trimmed
            .strip_prefix("__")
            .and_then(|value| value.strip_suffix("__"))
        {
            protocol = marker.to_owned();
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("sl") {
            continue;
        }
        let fields: Vec<_> = trimmed.split_whitespace().collect();
        if fields.len() >= 4 {
            observations.push(NetworkObservation {
                protocol: protocol.clone(),
                local_address: decode_endpoint(fields[1], protocol.ends_with('6')),
                remote_address: decode_endpoint(fields[2], protocol.ends_with('6')),
                state: network_state(fields[3]).to_owned(),
            });
        }
    }
    observations
}

fn decode_endpoint(raw: &str, ipv6: bool) -> String {
    let Some((address, port)) = raw.split_once(':') else {
        return raw.to_owned();
    };
    let Ok(port) = u16::from_str_radix(port, 16) else {
        return raw.to_owned();
    };
    if ipv6 {
        let Some(address) = decode_ipv6_address(address) else {
            return raw.to_owned();
        };
        return format!("[{address}]:{port}");
    }
    let Ok(value) = u32::from_str_radix(address, 16) else {
        return raw.to_owned();
    };
    let octets = value.to_le_bytes();
    format!(
        "{}.{}.{}.{}:{port}",
        octets[0], octets[1], octets[2], octets[3]
    )
}

fn decode_ipv6_address(raw: &str) -> Option<Ipv6Addr> {
    if raw.len() != 32 || !raw.is_ascii() {
        return None;
    }

    let mut octets = [0_u8; 16];
    for (index, chunk) in raw.as_bytes().chunks_exact(8).enumerate() {
        let chunk = std::str::from_utf8(chunk).ok()?;
        let word = u32::from_str_radix(chunk, 16).ok()?;
        octets[index * 4..index * 4 + 4].copy_from_slice(&word.to_ne_bytes());
    }
    Some(Ipv6Addr::from(octets))
}

fn network_state(code: &str) -> &str {
    match code {
        "01" => "established",
        "02" => "syn-sent",
        "03" => "syn-received",
        "04" => "fin-wait-1",
        "05" => "fin-wait-2",
        "06" => "time-wait",
        "07" => "closed",
        "08" => "close-wait",
        "09" => "last-ack",
        "0A" => "listening",
        "0B" => "closing",
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_busybox_process_output() {
        let raw = "PID   PPID  STAT COMMAND          COMMAND\n    1      0 S    sh               /bin/sh -c sleep 3600\n   12      1 R    demo             ./demo --verbose\n";
        assert_eq!(
            parse_processes(raw),
            vec![
                ProcessObservation {
                    pid: 1,
                    parent_pid: 0,
                    state: "S".into(),
                    command: "sh".into(),
                    arguments: "/bin/sh -c sleep 3600".into()
                },
                ProcessObservation {
                    pid: 12,
                    parent_pid: 1,
                    state: "R".into(),
                    command: "demo".into(),
                    arguments: "./demo --verbose".into()
                },
            ]
        );
    }

    #[test]
    fn parses_file_paths_containing_pipes() {
        let raw = "/workspace/a|b.txt|42|1720000000|regular file\n";
        assert_eq!(
            parse_files(raw),
            vec![FileObservation {
                path: "/workspace/a|b.txt".into(),
                size_bytes: 42,
                modified_unix_seconds: 1_720_000_000,
                kind: "regular file".into(),
            }]
        );
    }

    #[test]
    fn parses_and_decodes_network_snapshot() {
        let raw = "__tcp__\n  sl  local_address rem_address   st\n   0: 0100007F:1F90 00000000:0000 0A\n__udp__\n   2: 00000000:0035 00000000:0000 07\n";
        assert_eq!(
            parse_network(raw),
            vec![
                NetworkObservation {
                    protocol: "tcp".into(),
                    local_address: "127.0.0.1:8080".into(),
                    remote_address: "0.0.0.0:0".into(),
                    state: "listening".into()
                },
                NetworkObservation {
                    protocol: "udp".into(),
                    local_address: "0.0.0.0:53".into(),
                    remote_address: "0.0.0.0:0".into(),
                    state: "closed".into()
                },
            ]
        );
    }

    #[test]
    fn parses_and_decodes_ipv6_network_snapshot() {
        let raw = "__tcp6__\n  sl  local_address rem_address   st\n   0: 00000000000000000000000001000000:1F90 B80D0120000000000000000001000000:01BB 01\n";
        assert_eq!(
            parse_network(raw),
            vec![NetworkObservation {
                protocol: "tcp6".into(),
                local_address: "[::1]:8080".into(),
                remote_address: "[2001:db8::1]:443".into(),
                state: "established".into(),
            }]
        );
    }

    #[test]
    fn preserves_malformed_ipv6_endpoints() {
        assert_eq!(
            decode_endpoint("000000000000000000000000GG000000:0050", true),
            "000000000000000000000000GG000000:0050"
        );
        assert_eq!(decode_endpoint("00000000:0050", true), "00000000:0050");
    }
}
