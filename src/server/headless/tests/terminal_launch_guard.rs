//! Guarded direct terminal admission and what happens to the stream when its
//! launch stops occupying the terminal. The process table is the test
//! runtime's foreground leader, which changes without any detection event.

use super::*;
use crate::detect::{Agent, AgentState};
use crate::platform::ProcessInstance;
use crate::protocol::terminal_launch_guard::{
    AdmissionAnswer, AdmissionMode, AdmissionRequest, ANSWER_KIND, LAUNCH_ENDED_CODE,
};
use crate::terminal::launch_incarnation::LaunchIncarnation;

const SHELL: u32 = 4000;
const LAUNCH_PROCESS: ProcessInstance = ProcessInstance {
    pid: 4242,
    started: Some(7),
};
const OTHER_PROCESS: ProcessInstance = ProcessInstance {
    pid: 4343,
    started: Some(8),
};
/// The launch's process id reused by a later process.
const REUSED_PID: ProcessInstance = ProcessInstance {
    pid: 4242,
    started: Some(99),
};
const NEVER_MINTED: &str = "launch_00000000000000000000000000000000";

struct Occupied {
    terminal: crate::terminal::TerminalId,
    terminal_string: String,
    public_pane: String,
    token: String,
    input: tokio::sync::mpsc::Receiver<bytes::Bytes>,
}

/// An admitted launch, claimed by its process, seen by detection and leading
/// the terminal's foreground.
fn occupy(server: &mut HeadlessServer, terminal: &crate::terminal::TerminalId) -> String {
    let (mut launch, _nonce) = LaunchIncarnation::admit(Agent::Pi);
    launch
        .claim(LAUNCH_PROCESS, SHELL, Some(SHELL))
        .expect("the shell's child claims the launch");
    assert!(launch.observe(Agent::Pi, LAUNCH_PROCESS));
    let token = launch.token().to_owned();
    let state = server.app.state.terminals.get_mut(terminal).unwrap();
    state.admit_launch_incarnation(Some(launch));
    state.set_detected_state(Some(Agent::Pi), AgentState::Idle);
    leader(server, terminal, Some(LAUNCH_PROCESS));
    token
}

fn leader(
    server: &HeadlessServer,
    terminal: &crate::terminal::TerminalId,
    process: Option<ProcessInstance>,
) {
    server
        .app
        .terminal_runtimes
        .get(terminal)
        .unwrap()
        .test_set_foreground_leader(process);
}

fn with_occupied_terminal(test: impl FnOnce(&mut HeadlessServer, Occupied)) {
    with_terminal_session_test_server(|server, terminal, terminal_string, public_pane| {
        let (runtime, input) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        server
            .app
            .terminal_runtimes
            .insert(terminal.clone(), runtime);
        let token = occupy(server, &terminal);
        test(
            server,
            Occupied {
                terminal,
                terminal_string,
                public_pane,
                token,
                input,
            },
        );
    });
}

fn admit(
    server: &mut HeadlessServer,
    client_id: u64,
    mode: AdmissionMode,
    target: &str,
    takeover: bool,
    expected: &str,
) -> bool {
    server.handle_server_event(ServerEvent::TerminalGuardedAdmission {
        client_id,
        request: Some(AdmissionRequest {
            mode,
            target: target.to_owned(),
            takeover,
            expected_launch_incarnation: expected.to_owned(),
        }),
    })
}

/// The admission answer among the control messages queued so far; every
/// other message is returned for the caller's assertions.
fn answer(control: &std::sync::mpsc::Receiver<Vec<u8>>) -> AdmissionAnswer {
    let bytes = control
        .recv_timeout(Duration::from_secs(5))
        .expect("an answer is queued");
    let ServerMessage::EndpointControl { kind, data } = read_server_message(bytes) else {
        panic!("the first control message is the answer");
    };
    assert_eq!(kind, ANSWER_KIND);
    serde_json::from_str(&data).unwrap()
}

fn assert_nothing_attached(server: &HeadlessServer, client_id: u64, occupied: &Occupied) {
    assert!(!server.clients.contains_key(&client_id));
    assert!(!server
        .terminal_attach_owners
        .contains_key(&occupied.terminal_string));
    assert!(!server
        .app
        .state
        .direct_attach_resize_locks
        .contains(&occupied.terminal));
}

const MODES: [AdmissionMode; 3] = [
    AdmissionMode::Attach,
    AdmissionMode::Control,
    AdmissionMode::Observe,
];

#[test]
fn the_expected_launch_is_admitted_in_every_mode() {
    for mode in MODES {
        with_occupied_terminal(|server, occupied| {
            let control = connect_pending_terminal_client_with_control_rx(server, 7);
            let target = match mode {
                AdmissionMode::Attach => occupied.terminal_string.clone(),
                _ => occupied.public_pane.clone(),
            };
            assert!(admit(server, 7, mode, &target, false, &occupied.token));
            let answer = answer(&control);
            assert!(answer.admitted, "{mode:?}: {answer:?}");
            let client = server.clients.get(&7).unwrap();
            assert_eq!(
                client.launch_guard.as_deref(),
                Some(occupied.token.as_str())
            );
            match mode {
                AdmissionMode::Observe => assert!(matches!(
                    &client.mode,
                    ClientConnectionMode::TerminalObserve { terminal_id }
                        if *terminal_id == occupied.terminal_string
                )),
                _ => {
                    assert!(matches!(
                        &client.mode,
                        ClientConnectionMode::TerminalAttach { terminal_id }
                            if *terminal_id == occupied.terminal_string
                    ));
                    assert_eq!(
                        server.terminal_attach_owners.get(&occupied.terminal_string),
                        Some(&7)
                    );
                }
            }
        });
    }
}

#[test]
fn every_refusal_names_its_reason_and_attaches_nothing() {
    type Change = fn(&mut HeadlessServer, &Occupied);
    let cases: [(&str, Change, &str); 7] = [
        (
            "another launch",
            |server, occupied| {
                // A later start's launch is the attested one now.
                occupy(server, &occupied.terminal);
            },
            "terminal_launch_mismatch",
        ),
        (
            "no occupant",
            |server, occupied| {
                let state = server
                    .app
                    .state
                    .terminals
                    .get_mut(&occupied.terminal)
                    .unwrap();
                state.set_detected_state(None, AgentState::Unknown);
            },
            "terminal_no_occupant",
        ),
        (
            "revoked",
            |server, occupied| {
                let state = server
                    .app
                    .state
                    .terminals
                    .get_mut(&occupied.terminal)
                    .unwrap();
                state.admit_launch_incarnation(None);
            },
            "terminal_launch_not_attested",
        ),
        (
            "claimed but never seen",
            |server, occupied| {
                let (mut launch, _) = LaunchIncarnation::admit(Agent::Pi);
                launch.claim(LAUNCH_PROCESS, SHELL, Some(SHELL)).unwrap();
                let state = server
                    .app
                    .state
                    .terminals
                    .get_mut(&occupied.terminal)
                    .unwrap();
                state.admit_launch_incarnation(Some(launch));
            },
            "terminal_launch_not_attested",
        ),
        (
            "replacement detection has not seen",
            |server, occupied| {
                leader(server, &occupied.terminal, Some(OTHER_PROCESS));
            },
            "terminal_launch_not_attested",
        ),
        (
            "same process id, later process",
            |server, occupied| {
                leader(server, &occupied.terminal, Some(REUSED_PID));
            },
            "terminal_launch_not_attested",
        ),
        (
            "unreadable foreground",
            |server, occupied| {
                leader(server, &occupied.terminal, None);
            },
            "terminal_launch_not_attested",
        ),
    ];
    for (name, change, code) in cases {
        for mode in MODES {
            with_occupied_terminal(|server, occupied| {
                change(server, &occupied);
                let mut occupied = occupied;
                let control = connect_pending_terminal_client_with_control_rx(server, 7);
                let target = match mode {
                    AdmissionMode::Attach => occupied.terminal_string.clone(),
                    _ => occupied.public_pane.clone(),
                };
                let expected = occupied.token.clone();
                assert!(!admit(server, 7, mode, &target, false, &expected), "{name}");
                let answer = answer(&control);
                assert!(!answer.admitted, "{name}");
                assert_eq!(answer.code.as_deref(), Some(code), "{name} {mode:?}");
                assert_nothing_attached(server, 7, &occupied);
                assert!(
                    occupied.input.try_recv().is_err(),
                    "{name}: nothing written"
                );
            });
        }
    }
}

#[test]
fn a_token_this_terminal_never_ran_is_refused_and_the_message_names_the_current_launch() {
    with_occupied_terminal(|server, occupied| {
        let control = connect_pending_terminal_client_with_control_rx(server, 7);
        let target = occupied.terminal_string.clone();
        assert!(!admit(
            server,
            7,
            AdmissionMode::Attach,
            &target,
            false,
            NEVER_MINTED
        ));
        let answer = answer(&control);
        assert_eq!(answer.code.as_deref(), Some("terminal_launch_mismatch"));
        assert!(answer.message.unwrap().contains(&occupied.token));
        assert_nothing_attached(server, 7, &occupied);
    });
}

#[test]
fn a_gone_terminal_is_refused_in_every_mode() {
    for mode in MODES {
        with_occupied_terminal(|server, occupied| {
            let control = connect_pending_terminal_client_with_control_rx(server, 7);
            let expected = occupied.token.clone();
            assert!(!admit(
                server,
                7,
                mode,
                "no-such-terminal",
                false,
                &expected
            ));
            let answer = answer(&control);
            assert_eq!(
                answer.code.as_deref(),
                Some("terminal_not_found"),
                "{mode:?}"
            );
            assert_nothing_attached(server, 7, &occupied);
        });
    }
}

#[test]
fn attach_accepts_only_a_terminal_id_as_control_does_a_pane() {
    with_occupied_terminal(|server, occupied| {
        let control = connect_pending_terminal_client_with_control_rx(server, 7);
        let target = occupied.public_pane.clone();
        let expected = occupied.token.clone();
        assert!(!admit(
            server,
            7,
            AdmissionMode::Attach,
            &target,
            false,
            &expected
        ));
        assert_eq!(answer(&control).code.as_deref(), Some("terminal_not_found"));
    });
}

#[test]
fn a_noncanonical_token_is_refused_by_the_server_too() {
    for malformed in ["", "launch_", " launch_0", "--takeover", "LAUNCH_0"] {
        with_occupied_terminal(|server, occupied| {
            let control = connect_pending_terminal_client_with_control_rx(server, 7);
            let target = occupied.terminal_string.clone();
            assert!(!admit(
                server,
                7,
                AdmissionMode::Attach,
                &target,
                false,
                malformed
            ));
            assert_eq!(
                answer(&control).code.as_deref(),
                Some("invalid_launch_incarnation"),
                "{malformed:?}"
            );
            assert_nothing_attached(server, 7, &occupied);
        });
    }
    with_occupied_terminal(|server, occupied| {
        let control = connect_pending_terminal_client_with_control_rx(server, 7);
        assert!(
            !server.handle_server_event(ServerEvent::TerminalGuardedAdmission {
                client_id: 7,
                request: None,
            })
        );
        assert_eq!(
            answer(&control).code.as_deref(),
            Some("invalid_launch_incarnation")
        );
        assert_nothing_attached(server, 7, &occupied);
    });
}

#[test]
fn a_refused_takeover_leaves_the_current_owner_attached() {
    with_occupied_terminal(|server, occupied| {
        connect_pending_terminal_client(server, 7);
        let target = occupied.terminal_string.clone();
        assert!(
            server.handle_server_event(ServerEvent::ClientAttachTerminal {
                client_id: 7,
                terminal_id: target.clone(),
                takeover: false,
            })
        );
        let control = connect_pending_terminal_client_with_control_rx(server, 8);
        assert!(!admit(
            server,
            8,
            AdmissionMode::Attach,
            &target,
            true,
            NEVER_MINTED
        ));
        assert_eq!(
            answer(&control).code.as_deref(),
            Some("terminal_launch_mismatch")
        );
        assert_eq!(server.terminal_attach_owners.get(&target), Some(&7));
        assert!(server.clients.contains_key(&7));
    });
}

#[test]
fn an_owner_the_admission_turns_away_is_told_why_and_never_gets_the_guard() {
    with_occupied_terminal(|server, occupied| {
        connect_pending_terminal_client(server, 7);
        let target = occupied.terminal_string.clone();
        let expected = occupied.token.clone();
        assert!(admit(
            server,
            7,
            AdmissionMode::Attach,
            &target,
            false,
            &expected
        ));
        let control = connect_pending_terminal_client_with_control_rx(server, 8);
        assert!(!admit(
            server,
            8,
            AdmissionMode::Attach,
            &target,
            false,
            &expected
        ));
        let reason = read_server_shutdown_reason(control.recv().unwrap()).unwrap();
        assert!(
            reason.contains("already has an attached client"),
            "{reason}"
        );
        assert!(!server.clients.contains_key(&8));
        assert_eq!(server.terminal_attach_owners.get(&target), Some(&7));
    });
}

/// The reason in the stream's closing message.
fn closed_reason(control: &std::sync::mpsc::Receiver<Vec<u8>>) -> String {
    loop {
        let bytes = control
            .recv_timeout(Duration::from_secs(5))
            .expect("the stream gets a close reason");
        if let ServerMessage::ServerShutdown { reason } = read_server_message(bytes) {
            return reason.expect("a reason");
        }
    }
}

#[test]
fn a_replacement_ends_a_guarded_stream_with_the_reason_in_every_mode() {
    for mode in MODES {
        for replaced_by in [Some(OTHER_PROCESS), Some(REUSED_PID), None] {
            with_occupied_terminal(|server, occupied| {
                let control = connect_pending_terminal_client_with_control_rx(server, 7);
                let target = occupied.terminal_string.clone();
                let expected = occupied.token.clone();
                assert!(admit(server, 7, mode, &target, false, &expected));
                assert!(answer(&control).admitted);

                // While the launch holds the terminal the check changes nothing.
                assert!(!server.end_guarded_streams_whose_launch_ended());
                assert!(server.clients.contains_key(&7));

                leader(server, &occupied.terminal, replaced_by);
                assert!(server.end_guarded_streams_whose_launch_ended());
                let reason = closed_reason(&control);
                assert!(
                    reason.starts_with(&format!("{LAUNCH_ENDED_CODE}: ")),
                    "{reason}"
                );
                assert!(reason.contains(&expected), "{reason}");
                assert_nothing_attached(server, 7, &occupied);
            });
        }
    }
}

#[test]
fn input_from_a_guarded_stream_never_reaches_a_replacement() {
    fn input(client_id: u64) -> ServerEvent {
        ServerEvent::ClientInput {
            client_id,
            data: b"typed".to_vec(),
        }
    }
    fn scroll(client_id: u64) -> ServerEvent {
        ServerEvent::ClientAttachScroll {
            client_id,
            source: AttachScrollSource::Wheel,
            direction: AttachScrollDirection::Up,
            lines: 1,
            column: Some(1),
            row: Some(1),
            modifiers: 0,
        }
    }
    fn mouse(client_id: u64) -> ServerEvent {
        ServerEvent::ClientAttachMouse {
            client_id,
            kind: protocol::ClientMouseKind::Down(protocol::ClientMouseButton::Left),
            position: protocol::ClientMousePosition::Cell { column: 1, row: 1 },
            geometry: None,
            modifiers: 0,
            lines: 1,
        }
    }
    fn image(client_id: u64) -> ServerEvent {
        ServerEvent::ClientClipboardImage {
            client_id,
            target: protocol::ClientClipboardImageTarget::DirectTerminal,
            extension: "png".to_owned(),
            data: b"image".to_vec(),
        }
    }
    let events: [fn(u64) -> ServerEvent; 4] = [input, scroll, mouse, image];
    for event in events {
        with_occupied_terminal(|server, mut occupied| {
            let control = connect_pending_terminal_client_with_control_rx(server, 7);
            let target = occupied.terminal_string.clone();
            let expected = occupied.token.clone();
            assert!(admit(
                server,
                7,
                AdmissionMode::Control,
                &target,
                false,
                &expected
            ));
            assert!(answer(&control).admitted);

            // The replacement leads before any periodic check runs.
            leader(server, &occupied.terminal, Some(OTHER_PROCESS));
            assert!(!server.handle_server_event(event(7)));
            let reason = closed_reason(&control);
            assert!(reason.starts_with(LAUNCH_ENDED_CODE), "{reason}");
            assert!(occupied.input.try_recv().is_err(), "nothing was written");
            assert_nothing_attached(server, 7, &occupied);
        });
    }
}

#[test]
fn input_from_a_guarded_stream_reaches_the_launch_while_it_holds_the_terminal() {
    with_occupied_terminal(|server, mut occupied| {
        connect_pending_terminal_client(server, 7);
        let target = occupied.terminal_string.clone();
        let expected = occupied.token.clone();
        assert!(admit(
            server,
            7,
            AdmissionMode::Control,
            &target,
            false,
            &expected
        ));
        server.handle_server_event(ServerEvent::ClientInput {
            client_id: 7,
            data: b"typed".to_vec(),
        });
        assert_eq!(occupied.input.try_recv().unwrap().as_ref(), b"typed");
        assert!(server.clients.contains_key(&7));
    });
}

/// Without the option a stream is exactly what it was: nothing checks a
/// launch at admission or later, and input keeps reaching whoever is there.
#[test]
fn an_unguarded_stream_ignores_launches_entirely() {
    with_occupied_terminal(|server, mut occupied| {
        let target = occupied.terminal_string.clone();
        connect_pending_terminal_client(server, 7);
        connect_pending_terminal_client(server, 8);
        // Already replaced at admission.
        leader(server, &occupied.terminal, Some(OTHER_PROCESS));
        assert!(
            server.handle_server_event(ServerEvent::ClientAttachTerminal {
                client_id: 7,
                terminal_id: target.clone(),
                takeover: false,
            })
        );
        assert!(
            server.handle_server_event(ServerEvent::ClientObserveTerminal {
                client_id: 8,
                target: target.clone(),
            })
        );
        assert!(server
            .clients
            .values()
            .all(|client| client.launch_guard.is_none()));

        assert!(!server.end_guarded_streams_whose_launch_ended());
        server.handle_server_event(ServerEvent::ClientInput {
            client_id: 7,
            data: b"typed".to_vec(),
        });
        assert_eq!(occupied.input.try_recv().unwrap().as_ref(), b"typed");
        assert!(server.clients.contains_key(&7));
        assert!(server.clients.contains_key(&8));
    });
}
