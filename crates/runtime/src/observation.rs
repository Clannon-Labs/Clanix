use std::{collections::BTreeMap, future::Future, net::Ipv6Addr, sync::Arc};

use serde::Serialize;

use crate::{
    activity::{ExecutionActivity, ExecutionEvent},
    environment::{Environment, TranscriptEntry, now_ms},
    error::RuntimeError,
    podman,
    system::{FileObservation, NetworkObservation, ProcessObservation},
};

const PROCESS_OBSERVER_MARKER: &str = "__clannon_observer_pid__";
const PROCESS_COMMAND: &str = r#"observer=$$; printf '__clannon_observer_pid__%s\n' "$observer"; exec ps -o pid,ppid,stat,comm,args"#;
const FILE_COMMAND: &str = "LC_ALL=C; export LC_ALL; find /workspace -mindepth 1 -maxdepth 4 -exec stat -c '%n|%s|%Y|%F' '{}' ';' 2>/dev/null | sort | head -201";
const NETWORK_COMMAND: &str =
    "for f in tcp tcp6 udp udp6; do echo __${f}__; cat /proc/net/$f 2>/dev/null; done";
const MAX_SNAPSHOT_FILES: usize = 200;

#[derive(Serialize)]
pub struct ObservationSnapshot {
    environment_id: String,
    captured_at_ms: u128,
    execution_events: Vec<ExecutionEvent>,
    execution_events_omitted: u64,
    transcript: Vec<TranscriptEntry>,
    processes: Vec<ProcessObservation>,
    files: Vec<FileObservation>,
    network: Vec<NetworkObservation>,
    warnings: Vec<String>,
}

pub(crate) struct ObservationState {
    next_capture_sequence: u64,
    processes: Option<BTreeMap<u32, ProcessObservation>>,
    files: Option<BTreeMap<String, FileObservation>>,
    network: Option<Vec<NetworkObservation>>,
}

struct SystemCapture {
    processes: Option<Vec<ProcessObservation>>,
    files: Option<FileSample>,
    network: Option<Vec<NetworkObservation>>,
    warnings: Vec<String>,
}

struct FileSample {
    files: Vec<FileObservation>,
    complete: bool,
}

impl ObservationState {
    pub(crate) fn new() -> Self {
        Self {
            next_capture_sequence: 1,
            processes: None,
            files: None,
            network: None,
        }
    }

    fn commit(&mut self, capture_sequence: u64, capture: &SystemCapture) -> Vec<ExecutionActivity> {
        let mut activities = Vec::new();

        if let Some(processes) = &capture.processes {
            let current = process_map(processes);
            if let Some(previous) = &self.processes {
                activities.extend(diff_processes(capture_sequence, previous, &current));
            }
            self.processes = Some(current);
        }

        if let Some(files) = &capture.files {
            if files.complete {
                let current = file_map(&files.files);
                if let Some(previous) = &self.files {
                    activities.extend(diff_files(capture_sequence, previous, &current));
                }
                self.files = Some(current);
            }
        }

        if let Some(network) = &capture.network {
            let mut current = network.clone();
            current.sort();
            if let Some(previous) = &self.network {
                activities.extend(diff_network(capture_sequence, previous, &current));
            }
            self.network = Some(current);
        }

        activities
    }

    fn take_capture_sequence(&mut self) -> u64 {
        let sequence = self.next_capture_sequence;
        self.next_capture_sequence = self
            .next_capture_sequence
            .checked_add(1)
            .expect("observation capture sequence exhausted");
        sequence
    }
}

pub(crate) async fn collect(environment: Arc<Environment>) -> ObservationSnapshot {
    let container = environment.container_name().to_owned();
    collect_with(environment, move |command| {
        let container = container.clone();
        async move { podman::exec(&container, command).await }
    })
    .await
}

async fn collect_with<Execute, ExecuteFuture>(
    environment: Arc<Environment>,
    mut execute: Execute,
) -> ObservationSnapshot
where
    Execute: FnMut(&'static str) -> ExecuteFuture,
    ExecuteFuture: Future<Output = Result<String, RuntimeError>>,
{
    let mut state = environment.observation_state().lock().await;
    let mut warnings = Vec::new();

    let processes = match execute(PROCESS_COMMAND).await {
        Ok(raw) => Some(parse_processes(&raw)),
        Err(error) => {
            warnings.push(error.into_message());
            None
        }
    };
    let files = match execute(FILE_COMMAND).await {
        Ok(raw) => {
            let sample = parse_file_sample(&raw);
            if !sample.complete {
                warnings.push(
                    "file snapshot is limited to 200 entries; file changes were not sampled"
                        .to_owned(),
                );
            }
            Some(sample)
        }
        Err(error) => {
            warnings.push(error.into_message());
            None
        }
    };
    let network = match execute(NETWORK_COMMAND).await {
        Ok(raw) => Some(parse_network(&raw)),
        Err(error) => {
            warnings.push(error.into_message());
            None
        }
    };
    let capture = SystemCapture {
        processes,
        files,
        network,
        warnings,
    };

    let transcript = environment.transcript_snapshot().await;
    let captured_at_ms = now_ms();
    let capture_sequence = state.take_capture_sequence();
    let activities = state.commit(capture_sequence, &capture);
    environment
        .activity()
        .record_batch(activities, captured_at_ms);
    let (execution_events, execution_events_omitted) = environment.activity_snapshot();

    ObservationSnapshot {
        environment_id: environment.id().to_owned(),
        captured_at_ms,
        execution_events,
        execution_events_omitted,
        transcript,
        processes: capture.processes.unwrap_or_default(),
        files: capture.files.map(|sample| sample.files).unwrap_or_default(),
        network: capture.network.unwrap_or_default(),
        warnings: capture.warnings,
    }
}

fn process_map(observations: &[ProcessObservation]) -> BTreeMap<u32, ProcessObservation> {
    observations
        .iter()
        .map(|process| (process.pid, process.clone()))
        .collect()
}

fn file_map(observations: &[FileObservation]) -> BTreeMap<String, FileObservation> {
    observations
        .iter()
        .map(|file| (file.path.clone(), file.clone()))
        .collect()
}

fn diff_processes(
    capture_sequence: u64,
    previous: &BTreeMap<u32, ProcessObservation>,
    current: &BTreeMap<u32, ProcessObservation>,
) -> Vec<ExecutionActivity> {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut changed = Vec::new();

    for (pid, process) in previous {
        match current.get(pid) {
            None => removed.push(ExecutionActivity::ProcessRemoved {
                capture_sequence,
                process: process.clone(),
            }),
            Some(current) if current != process => {
                changed.push(ExecutionActivity::ProcessChanged {
                    capture_sequence,
                    previous: process.clone(),
                    current: current.clone(),
                });
            }
            Some(_) => {}
        }
    }
    for (pid, process) in current {
        if !previous.contains_key(pid) {
            added.push(ExecutionActivity::ProcessAdded {
                capture_sequence,
                process: process.clone(),
            });
        }
    }
    removed.extend(added);
    removed.extend(changed);
    removed
}

fn diff_files(
    capture_sequence: u64,
    previous: &BTreeMap<String, FileObservation>,
    current: &BTreeMap<String, FileObservation>,
) -> Vec<ExecutionActivity> {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut changed = Vec::new();

    for (path, file) in previous {
        match current.get(path) {
            None => removed.push(ExecutionActivity::FileRemoved {
                capture_sequence,
                file: file.clone(),
            }),
            Some(current) if current != file => changed.push(ExecutionActivity::FileChanged {
                capture_sequence,
                previous: file.clone(),
                current: current.clone(),
            }),
            Some(_) => {}
        }
    }
    for (path, file) in current {
        if !previous.contains_key(path) {
            added.push(ExecutionActivity::FileAdded {
                capture_sequence,
                file: file.clone(),
            });
        }
    }
    removed.extend(added);
    removed.extend(changed);
    removed
}

fn diff_network(
    capture_sequence: u64,
    previous: &[NetworkObservation],
    current: &[NetworkObservation],
) -> Vec<ExecutionActivity> {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut previous_index = 0;
    let mut current_index = 0;

    while previous_index < previous.len() && current_index < current.len() {
        match previous[previous_index].cmp(&current[current_index]) {
            std::cmp::Ordering::Less => {
                removed.push(ExecutionActivity::NetworkRemoved {
                    capture_sequence,
                    network: previous[previous_index].clone(),
                });
                previous_index += 1;
            }
            std::cmp::Ordering::Greater => {
                added.push(ExecutionActivity::NetworkAdded {
                    capture_sequence,
                    network: current[current_index].clone(),
                });
                current_index += 1;
            }
            std::cmp::Ordering::Equal => {
                previous_index += 1;
                current_index += 1;
            }
        }
    }
    for network in &previous[previous_index..] {
        removed.push(ExecutionActivity::NetworkRemoved {
            capture_sequence,
            network: network.clone(),
        });
    }
    for network in &current[current_index..] {
        added.push(ExecutionActivity::NetworkAdded {
            capture_sequence,
            network: network.clone(),
        });
    }
    removed.extend(added);
    removed
}

fn parse_processes(raw: &str) -> Vec<ProcessObservation> {
    let observer_pid = raw.lines().find_map(|line| {
        line.trim()
            .strip_prefix(PROCESS_OBSERVER_MARKER)
            .and_then(|pid| pid.parse::<u32>().ok())
    });
    let mut processes: Vec<_> = raw
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent_pid = fields.next()?.parse().ok()?;
            let state = fields.next()?.to_owned();
            let command = fields.next()?.to_owned();
            let arguments = fields.collect::<Vec<_>>().join(" ");
            (Some(pid) != observer_pid).then_some(ProcessObservation {
                pid,
                parent_pid,
                state,
                command,
                arguments,
            })
        })
        .collect();
    processes.sort_by_key(|process| process.pid);
    processes
}

fn parse_file_sample(raw: &str) -> FileSample {
    let complete = raw.lines().count() <= MAX_SNAPSHOT_FILES;
    let mut files = parse_files(raw);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files.truncate(MAX_SNAPSHOT_FILES);
    FileSample { files, complete }
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
    observations.sort();
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
    use std::{
        fmt::Write as _,
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tokio::{
        sync::{Notify, oneshot},
        time::{Duration, timeout},
    };

    use super::*;

    fn process(pid: u32, state: &str) -> ProcessObservation {
        ProcessObservation {
            pid,
            parent_pid: 1,
            state: state.into(),
            command: format!("process-{pid}"),
            arguments: format!("process-{pid} --test"),
        }
    }

    fn process_with(pid: u32, command: &str, arguments: &str) -> ProcessObservation {
        ProcessObservation {
            pid,
            parent_pid: 1,
            state: "S".into(),
            command: command.into(),
            arguments: arguments.into(),
        }
    }

    fn file(path: &str, size_bytes: u64) -> FileObservation {
        FileObservation {
            path: path.into(),
            size_bytes,
            modified_unix_seconds: 1,
            kind: "regular file".into(),
        }
    }

    fn network(port: u16, state: &str) -> NetworkObservation {
        NetworkObservation {
            protocol: "tcp".into(),
            local_address: format!("127.0.0.1:{port}"),
            remote_address: "0.0.0.0:0".into(),
            state: state.into(),
        }
    }

    fn capture(
        processes: Option<Vec<ProcessObservation>>,
        files: Option<(Vec<FileObservation>, bool)>,
        network: Option<Vec<NetworkObservation>>,
    ) -> SystemCapture {
        SystemCapture {
            processes,
            files: files.map(|(files, complete)| FileSample { files, complete }),
            network,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn diffs_domains_in_canonical_order() {
        let mut state = ObservationState::new();
        let first = capture(
            Some(vec![process(2, "S"), process(1, "S")]),
            Some((vec![file("/workspace/b", 1), file("/workspace/a", 1)], true)),
            Some(vec![
                network(10, "listening"),
                network(10, "listening"),
                network(20, "listening"),
            ]),
        );
        assert!(state.commit(1, &first).is_empty());

        let second = capture(
            Some(vec![process(3, "S"), process(1, "R")]),
            Some((vec![file("/workspace/c", 1), file("/workspace/a", 2)], true)),
            Some(vec![network(10, "listening"), network(30, "listening")]),
        );
        let changes = state.commit(2, &second);

        assert_eq!(
            changes,
            vec![
                ExecutionActivity::ProcessRemoved {
                    capture_sequence: 2,
                    process: process(2, "S"),
                },
                ExecutionActivity::ProcessAdded {
                    capture_sequence: 2,
                    process: process(3, "S"),
                },
                ExecutionActivity::ProcessChanged {
                    capture_sequence: 2,
                    previous: process(1, "S"),
                    current: process(1, "R"),
                },
                ExecutionActivity::FileRemoved {
                    capture_sequence: 2,
                    file: file("/workspace/b", 1),
                },
                ExecutionActivity::FileAdded {
                    capture_sequence: 2,
                    file: file("/workspace/c", 1),
                },
                ExecutionActivity::FileChanged {
                    capture_sequence: 2,
                    previous: file("/workspace/a", 1),
                    current: file("/workspace/a", 2),
                },
                ExecutionActivity::NetworkRemoved {
                    capture_sequence: 2,
                    network: network(10, "listening"),
                },
                ExecutionActivity::NetworkRemoved {
                    capture_sequence: 2,
                    network: network(20, "listening"),
                },
                ExecutionActivity::NetworkAdded {
                    capture_sequence: 2,
                    network: network(30, "listening"),
                },
            ]
        );
    }

    #[test]
    fn failures_and_truncation_preserve_the_last_complete_baselines() {
        let mut state = ObservationState::new();
        let first = capture(
            Some(vec![process(1, "S")]),
            Some((vec![file("/workspace/a", 1)], true)),
            Some(vec![network(10, "listening")]),
        );
        assert!(state.commit(1, &first).is_empty());
        assert!(state.commit(2, &capture(None, None, None)).is_empty());
        assert!(
            state
                .commit(
                    3,
                    &capture(
                        None,
                        Some((vec![file("/workspace/hidden", 9)], false)),
                        None,
                    ),
                )
                .is_empty()
        );

        let changes = state.commit(
            4,
            &capture(
                Some(vec![process(1, "R")]),
                Some((vec![file("/workspace/b", 1)], true)),
                Some(vec![network(20, "listening")]),
            ),
        );
        assert!(matches!(
            changes[0],
            ExecutionActivity::ProcessChanged {
                capture_sequence: 4,
                ..
            }
        ));
        assert!(matches!(
            changes[1],
            ExecutionActivity::FileRemoved {
                capture_sequence: 4,
                ..
            }
        ));
        assert!(matches!(
            changes[2],
            ExecutionActivity::FileAdded {
                capture_sequence: 4,
                ..
            }
        ));
        assert!(matches!(
            changes[3],
            ExecutionActivity::NetworkRemoved {
                capture_sequence: 4,
                ..
            }
        ));
        assert!(matches!(
            changes[4],
            ExecutionActivity::NetworkAdded {
                capture_sequence: 4,
                ..
            }
        ));
    }

    #[test]
    fn filters_the_observer_process_and_sorts_by_pid() {
        let raw = "__clannon_observer_pid__12\nPID PPID STAT COMMAND COMMAND\n  12 1 R ps ps -o pid\n  20 1 S demo demo\n  3 1 S sh /bin/sh\n";
        assert_eq!(
            parse_processes(raw),
            vec![
                process_with(3, "sh", "/bin/sh"),
                process_with(20, "demo", "demo"),
            ]
        );
    }

    #[test]
    fn file_sample_exposes_two_hundred_but_marks_two_hundred_one_incomplete() {
        let mut raw = String::new();
        for index in 0..=MAX_SNAPSHOT_FILES {
            writeln!(raw, "/workspace/{index:03}|1|1|regular file").unwrap();
        }
        let sample = parse_file_sample(&raw);
        assert_eq!(sample.files.len(), MAX_SNAPSHOT_FILES);
        assert!(!sample.complete);
        assert_eq!(sample.files.first().unwrap().path, "/workspace/000");
        assert_eq!(sample.files.last().unwrap().path, "/workspace/199");
    }

    #[tokio::test]
    async fn concurrent_refreshes_are_serialized_per_environment() {
        let environment = Environment::for_test();
        let first_environment = environment.clone();
        let release = Arc::new(Notify::new());
        let first_release = release.clone();
        let (started_sender, started_receiver) = oneshot::channel();
        let started_sender = Arc::new(StdMutex::new(Some(started_sender)));
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = calls.clone();
        let first = tokio::spawn(async move {
            collect_with(first_environment, move |_| {
                let call = first_calls.fetch_add(1, Ordering::SeqCst);
                let release = first_release.clone();
                let started_sender = started_sender.clone();
                async move {
                    if call == 0 {
                        if let Some(sender) = started_sender.lock().unwrap().take() {
                            let _ = sender.send(());
                        }
                        release.notified().await;
                    }
                    Ok(String::new())
                }
            })
            .await
        });
        started_receiver.await.unwrap();

        let second_environment = environment.clone();
        let second_calls = Arc::new(AtomicUsize::new(0));
        let observed_second_calls = second_calls.clone();
        let second = tokio::spawn(async move {
            collect_with(second_environment, move |_| {
                second_calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(String::new()))
            })
            .await
        });
        tokio::task::yield_now().await;
        assert_eq!(observed_second_calls.load(Ordering::SeqCst), 0);

        release.notify_one();
        timeout(Duration::from_secs(1), first)
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed_second_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn parses_busybox_process_output_without_a_marker() {
        let raw = "PID   PPID  STAT COMMAND          COMMAND\n    1      0 S    sh               /bin/sh -c sleep 3600\n   12      1 R    demo             ./demo --verbose\n";
        assert_eq!(
            parse_processes(raw),
            vec![
                ProcessObservation {
                    pid: 1,
                    parent_pid: 0,
                    state: "S".into(),
                    command: "sh".into(),
                    arguments: "/bin/sh -c sleep 3600".into(),
                },
                ProcessObservation {
                    pid: 12,
                    parent_pid: 1,
                    state: "R".into(),
                    command: "demo".into(),
                    arguments: "./demo --verbose".into(),
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
                    state: "listening".into(),
                },
                NetworkObservation {
                    protocol: "udp".into(),
                    local_address: "0.0.0.0:53".into(),
                    remote_address: "0.0.0.0:0".into(),
                    state: "closed".into(),
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
