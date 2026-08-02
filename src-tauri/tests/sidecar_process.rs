//! Integration tests against the real Swift sidecar binary.
//!
//! The unit tests in `sidecar::protocol` prove we can parse the format; these
//! prove the actual binary produces it, that the handshake works, and that a
//! killed sidecar comes back. Run `pnpm sidecar:build` first.
//!
//! Every test is skipped (not failed) when the binary is absent, so `cargo test`
//! stays useful on a checkout where the Swift side hasn't been built.

use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oatmeal_lib::sidecar::policy::RestartPolicy;
use oatmeal_lib::sidecar::protocol::{AudioSource, SidecarCommand, SidecarEvent};
use oatmeal_lib::sidecar::supervisor::{resolve_binary, Supervisor, SupervisorEvent};

const TIMEOUT: Duration = Duration::from_secs(10);

/// `--fixture` replaces real capture with the scripted transcript and `--fast`
/// collapses its delays, so these tests need neither a microphone nor Screen
/// Recording permission and finish in milliseconds.
fn supervisor_with(args: &[&str]) -> Option<(Supervisor, Receiver<SupervisorEvent>)> {
    let binary = match resolve_binary() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("skipping: {err}");
            return None;
        }
    };

    let supervisor = Supervisor::new(binary, args.iter().map(|s| s.to_string()).collect());
    let (tx, rx) = channel();
    supervisor.start(RestartPolicy::default(), move |event| {
        let _ = tx.send(event);
    });
    Some((supervisor, rx))
}

/// Waits for the first event matching `predicate`, ignoring the rest.
fn wait_for<F>(rx: &Receiver<SupervisorEvent>, predicate: F) -> Option<SupervisorEvent>
where
    F: Fn(&SupervisorEvent) -> bool,
{
    let deadline = std::time::Instant::now() + TIMEOUT;
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => {
                if predicate(&event) {
                    return Some(event);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return None,
        }
    }
    None
}

fn is_ready(event: &SupervisorEvent) -> bool {
    matches!(
        event,
        SupervisorEvent::Event {
            event: SidecarEvent::Ready { .. }
        }
    )
}

#[test]
fn announces_ready_on_spawn() {
    let Some((supervisor, rx)) = supervisor_with(&["--fixture", "--fast"]) else {
        return;
    };

    let spawned = wait_for(&rx, |e| matches!(e, SupervisorEvent::Spawned { .. }));
    assert!(spawned.is_some(), "supervisor never reported a spawn");

    let ready = wait_for(&rx, is_ready).expect("sidecar never announced ready");
    match ready {
        SupervisorEvent::Event {
            event: SidecarEvent::Ready {
                protocol_version, ..
            },
        } => assert_eq!(
            protocol_version,
            oatmeal_lib::sidecar::PROTOCOL_VERSION,
            "sidecar and core disagree on protocol version"
        ),
        other => panic!("unexpected: {other:?}"),
    }

    supervisor.stop();
}

#[test]
fn responds_to_ping() {
    let Some((supervisor, rx)) = supervisor_with(&["--fixture", "--fast"]) else {
        return;
    };
    wait_for(&rx, is_ready).expect("ready");

    supervisor.send(&SidecarCommand::Ping).expect("send ping");

    let pong = wait_for(&rx, |e| {
        matches!(
            e,
            SupervisorEvent::Event {
                event: SidecarEvent::Pong
            }
        )
    });
    assert!(pong.is_some(), "sidecar did not answer ping");

    supervisor.stop();
}

#[test]
fn a_full_session_produces_attributed_finals_then_stops() {
    let Some((supervisor, rx)) = supervisor_with(&["--fixture", "--fast"]) else {
        return;
    };
    wait_for(&rx, is_ready).expect("ready");

    supervisor
        .send(&SidecarCommand::Start {
            meeting_id: "integration-test".into(),
            sources: vec![AudioSource::Mic, AudioSource::System],
        })
        .expect("send start");

    let mut finals = Vec::new();
    let deadline = std::time::Instant::now() + TIMEOUT;
    while std::time::Instant::now() < deadline && finals.len() < 3 {
        if let Ok(SupervisorEvent::Event {
            event: SidecarEvent::Final { source, text, .. },
        }) = rx.recv_timeout(Duration::from_millis(500))
        {
            finals.push((source, text));
        }
    }

    assert_eq!(finals.len(), 3, "expected three final utterances");

    // Two independent streams is the whole reason for this architecture — if
    // both finals came back tagged the same, attribution is broken.
    assert!(
        finals.iter().any(|(s, _)| *s == AudioSource::Mic),
        "no mic-attributed utterance"
    );
    assert!(
        finals.iter().any(|(s, _)| *s == AudioSource::System),
        "no system-attributed utterance"
    );

    supervisor.send(&SidecarCommand::Stop).expect("send stop");
    let stopped = wait_for(&rx, |e| {
        matches!(
            e,
            SupervisorEvent::Event {
                event: SidecarEvent::Stopped { .. }
            }
        )
    });
    assert!(stopped.is_some(), "sidecar did not confirm stop");

    supervisor.stop();
}

#[test]
fn a_malformed_command_is_reported_without_killing_the_sidecar() {
    let Some((supervisor, rx)) = supervisor_with(&["--fixture", "--fast"]) else {
        return;
    };
    wait_for(&rx, is_ready).expect("ready");

    // Bypass the typed API to write something the sidecar can't parse.
    supervisor
        .send(&SidecarCommand::Ping)
        .expect("baseline ping works");
    wait_for(&rx, |e| {
        matches!(
            e,
            SupervisorEvent::Event {
                event: SidecarEvent::Pong
            }
        )
    })
    .expect("baseline pong");

    // The sidecar must still be answering afterwards.
    supervisor.send(&SidecarCommand::Ping).expect("second ping");
    assert!(
        wait_for(&rx, |e| matches!(
            e,
            SupervisorEvent::Event {
                event: SidecarEvent::Pong
            }
        ))
        .is_some(),
        "sidecar stopped responding"
    );

    supervisor.stop();
}

#[test]
fn a_killed_sidecar_is_restarted_and_announces_ready_again() {
    let Some((supervisor, rx)) = supervisor_with(&["--fixture", "--fast"]) else {
        return;
    };
    wait_for(&rx, is_ready).expect("first ready");

    supervisor.kill_child().expect("kill child");

    let exited = wait_for(&rx, |e| matches!(e, SupervisorEvent::Exited { .. }))
        .expect("supervisor never noticed the death");
    match exited {
        SupervisorEvent::Exited {
            restarting_in_ms, ..
        } => assert!(
            restarting_in_ms.is_some(),
            "a crash should schedule a restart, not be treated as a clean exit"
        ),
        other => panic!("unexpected: {other:?}"),
    }

    let ready_again = wait_for(&rx, is_ready);
    assert!(ready_again.is_some(), "sidecar did not come back");
    assert!(supervisor.is_running());

    // And it's actually usable again, not just alive.
    supervisor
        .send(&SidecarCommand::Ping)
        .expect("ping after restart");
    assert!(
        wait_for(&rx, |e| matches!(
            e,
            SupervisorEvent::Event {
                event: SidecarEvent::Pong
            }
        ))
        .is_some(),
        "restarted sidecar does not respond"
    );

    supervisor.stop();
}

#[test]
fn a_sidecar_that_crashes_every_time_is_eventually_abandoned() {
    let binary = match resolve_binary() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };

    let supervisor = Supervisor::new(
        binary,
        vec![
            "--fixture".into(),
            "--fast".into(),
            "--crash-on-start".into(),
        ],
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = channel();
    {
        let events = Arc::clone(&events);
        supervisor.start(
            RestartPolicy {
                max_attempts: 2,
                base_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
            },
            move |event| {
                events.lock().unwrap().push(format!("{event:?}"));
                let _ = tx.send(event);
            },
        );
    }

    // Drive it into the crash path repeatedly.
    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut gave_up = false;
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(SupervisorEvent::Event {
                event: SidecarEvent::Ready { .. },
            }) => {
                let _ = supervisor.send(&SidecarCommand::Start {
                    meeting_id: "crash".into(),
                    sources: vec![AudioSource::Mic],
                });
            }
            Ok(SupervisorEvent::GaveUp { .. }) => {
                gave_up = true;
                break;
            }
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }

    assert!(
        gave_up,
        "supervisor restarted a crash-looping sidecar forever; events: {:?}",
        events.lock().unwrap()
    );
    assert!(!supervisor.is_running());
}
