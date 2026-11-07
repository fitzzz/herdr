//! Launch incarnations through the real request parser and app handlers: an
//! admitted start, the claim its typed helper sends from its own process,
//! detection's view of the agent process, and `agent.send_keys` guarded by
//! the token.

use bytes::Bytes;

use crate::api::schema::Request;
use crate::app::App;
use crate::detect::Agent;
use crate::platform::{AncestorProcess, ForegroundProcess, ProcessInstance};

/// The pane shell's process id while a claim is read. The test runtime reads
/// no live process otherwise, so it is set only around the claim.
const SHELL: u32 = 4000;

struct Launch {
    pane: crate::layout::PaneId,
    public_pane: String,
    terminal_id: crate::terminal::TerminalId,
    token: String,
    nonce: String,
}

fn app() -> App {
    let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        &crate::config::Config::default(),
        crate::app::AppPolicy::TEST,
        None,
        api_rx,
        crate::api::EventHub::default(),
    );
    app.state.workspaces = vec![crate::workspace::Workspace::test_new("launch")];
    app.state.ensure_test_terminals();
    app.state.active = Some(0);
    app.state.selected = 0;
    app.state.mode = crate::app::Mode::Terminal;
    app
}

/// A JSON request as a client writes it, through the same parser the server
/// reads the wire with.
fn request(app: &mut App, json: serde_json::Value) -> serde_json::Value {
    let request: Request = serde_json::from_str(&json.to_string()).expect("a valid request");
    serde_json::from_str(&app.handle_api_request(request)).unwrap()
}

fn process(pid: u32, started: u64) -> ProcessInstance {
    ProcessInstance {
        pid,
        started: Some(started),
    }
}

/// The nonce in the line Herdr typed, as the shell would run it.
fn typed_nonce(typed: &str) -> String {
    let (_, after) = typed
        .split_once(crate::terminal::launch_incarnation::ATTEST_LAUNCH_COMMAND)
        .unwrap_or_else(|| panic!("the start types the attesting helper: {typed}"));
    after.split_whitespace().next().unwrap().to_string()
}

/// Admits `agent.start` of `kind` named `name` into the root pane, whose
/// runtime records what is typed into it.
fn start(
    app: &mut App,
    name: &str,
    kind: &str,
    args: &[&str],
) -> (Launch, tokio::sync::mpsc::Receiver<Bytes>) {
    let pane = app.state.workspaces[0].tabs[0].root_pane;
    let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane]
        .attached_terminal_id
        .clone();
    let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
    app.terminal_runtimes.insert(terminal_id.clone(), runtime);
    let public_pane = app.public_pane_id(0, pane).unwrap();
    let reply = request(
        app,
        serde_json::json!({"id": "start", "method": "agent.start", "params": {
            "name": name, "kind": kind, "pane_id": public_pane, "args": args
        }}),
    );
    assert_eq!(reply["result"]["type"], "agent_started", "{reply}");
    let token = reply["result"]["launch_incarnation"]
        .as_str()
        .expect("the start reply carries the admission's token")
        .to_string();
    assert!(
        reply["result"]["agent"].get("launch_incarnation").is_none(),
        "nothing attests the launch before its process claimed it: {reply}"
    );
    let typed = String::from_utf8(rx.try_recv().unwrap().to_vec()).unwrap();
    assert!(
        typed.contains(&format!(
            " -- {}",
            crate::detect::interactive_agent_executable(
                crate::detect::parse_agent_label(kind).unwrap()
            )
        )),
        "the helper runs the agent's own argv: {typed}"
    );
    assert!(
        typed.starts_with(&format!(
            "case $(command -v {} 2>/dev/null) in /*) ",
            crate::detect::interactive_agent_executable(
                crate::detect::parse_agent_label(kind).unwrap()
            )
        )),
        "the shell resolves the command first: {typed}"
    );
    assert!(typed.contains(" --resolved-by-shell; "), "{typed}");
    let nonce = typed_nonce(&typed);
    assert!(!typed.contains(&token), "the public token is not typed");
    (
        Launch {
            pane,
            public_pane,
            terminal_id,
            token,
            nonce,
        },
        rx,
    )
}

/// The helper's claim, sent from `sender` whose parent is `parent`.
fn claim(
    app: &mut App,
    launch: &Launch,
    nonce: &str,
    sender: Option<(ProcessInstance, u32)>,
) -> serde_json::Value {
    app.terminal_runtimes
        .get(&launch.terminal_id)
        .unwrap()
        .test_set_child_pid(SHELL);
    let ancestry = sender.map(|(process, parent)| {
        vec![AncestorProcess {
            process: ForegroundProcess {
                pid: process.pid,
                name: "herdr".into(),
                argv0: None,
                argv: None,
                cmdline: None,
                started: process.started,
            },
            parent,
            process_group: Some(process.pid),
        }]
    });
    let request: Request = serde_json::from_value(serde_json::json!({
        "id": "attest", "method": "agent.attest_launch", "params": {"nonce": nonce}
    }))
    .unwrap();
    let response = app.handle_api_request_after_internal_events_drained(request, ancestry);
    app.terminal_runtimes
        .get(&launch.terminal_id)
        .unwrap()
        .test_set_child_pid(0);
    serde_json::from_str(&response).unwrap()
}

/// Makes `process` the one the process table shows leading the pane's
/// foreground process group now, whether or not detection has seen it.
fn foreground(app: &App, launch: &Launch, process: Option<ProcessInstance>) {
    app.terminal_runtimes
        .get(&launch.terminal_id)
        .unwrap()
        .test_set_foreground_leader(process);
}

/// Detection identified `process` running `agent` in the launch's pane,
/// where it leads the foreground.
fn observe(app: &mut App, launch: &Launch, agent: Agent, process: ProcessInstance) {
    foreground(app, launch, Some(process));
    app.handle_internal_event(crate::events::AppEvent::AgentProcessDetected {
        pane_id: launch.pane,
        agent,
        observed_at: std::time::Instant::now(),
    });
    app.handle_internal_event(crate::events::AppEvent::AgentLaunchObserved {
        pane_id: launch.pane,
        agent,
        process,
        argv: None,
        account: None,
    });
}

/// The start, its claim from process 4242 and detection of that process.
fn attested(app: &mut App, kind: Agent) -> (Launch, tokio::sync::mpsc::Receiver<Bytes>) {
    let (launch, rx) = start(app, "worker", crate::detect::agent_label(kind), &[]);
    let claimed = claim(app, &launch, &launch.nonce, Some((process(4242, 7), SHELL)));
    assert_eq!(claimed["result"]["type"], "ok", "{claimed}");
    observe(app, &launch, kind, process(4242, 7));
    // The managed start finished: the agent is idle past its settle delay.
    let terminal = app.state.terminals.get_mut(&launch.terminal_id).unwrap();
    terminal.set_detected_state(Some(kind), crate::detect::AgentState::Idle);
    assert!(terminal.reconcile_managed_agent_at(
        std::time::Instant::now() + crate::app::AGENT_START_SETTLE_DELAY,
        false
    ));
    assert!(terminal.managed_agent_interactive_ready());
    assert_eq!(
        current(app, &launch).as_deref(),
        Some(launch.token.as_str())
    );
    (launch, rx)
}

/// What `agent.get` attests now.
fn current(app: &mut App, launch: &Launch) -> Option<String> {
    let reply = request(
        app,
        serde_json::json!({"id": "get", "method": "agent.get", "params": {"target": launch.public_pane}}),
    );
    assert_eq!(
        reply["result"]["type"], "agent_info",
        "the agent is still there to ask: {reply}"
    );
    reply["result"]["agent"]["launch_incarnation"]
        .as_str()
        .map(str::to_string)
}

/// What the terminal itself holds, also where no agent is left to ask.
fn held(app: &App, launch: &Launch) -> Option<String> {
    app.state.terminals[&launch.terminal_id]
        .launch_incarnation()
        .map(str::to_string)
}

/// Sends Esc as the server dispatches it: a send guarded by a launch is
/// answered once the terminal writer has checked the launch's process.
fn send_esc(app: &mut App, launch: &Launch, guards: serde_json::Value) -> serde_json::Value {
    let mut params = serde_json::json!({"target": launch.public_pane, "keys": ["esc"]});
    for (key, value) in guards.as_object().unwrap() {
        params[key] = value.clone();
    }
    let json = serde_json::json!({"id": "esc", "method": "agent.send_keys", "params": params});
    let request: Request = serde_json::from_str(&json.to_string()).expect("a valid request");
    let guarded = matches!(
        &request.method,
        crate::api::schema::Method::AgentSendKeys(params)
            if params.expected_launch_incarnation.is_some()
    );
    if !guarded {
        return serde_json::from_str(&app.handle_api_request(request)).unwrap();
    }
    let (respond_to, response) = std::sync::mpsc::channel();
    app.handle_deferred_agent_send_keys(request, respond_to);
    serde_json::from_str(&response.recv().expect("the writer answers")).unwrap()
}

fn assert_written(response: &serde_json::Value, rx: &mut tokio::sync::mpsc::Receiver<Bytes>) {
    assert_eq!(response["result"]["type"], "ok", "{response}");
    assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\x1b"));
    assert!(rx.try_recv().is_err());
}

fn assert_refused(
    response: &serde_json::Value,
    code: &str,
    rx: &mut tokio::sync::mpsc::Receiver<Bytes>,
) {
    assert_eq!(response["error"]["code"], code, "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no keys were sent"),
        "{response}"
    );
    assert!(rx.try_recv().is_err(), "a refusal writes nothing");
}

fn exit(app: &mut App, launch: &Launch, agent: Agent) {
    app.handle_internal_event(crate::events::AppEvent::StateChanged {
        pane_id: launch.pane,
        agent: Some(agent),
        state: crate::detect::AgentState::Idle,
        visible_blocker: false,
        visible_working: false,
        process_exited: true,
        observed_at: std::time::Instant::now(),
    });
}

#[tokio::test]
async fn the_token_is_attested_only_once_detection_sees_the_claimed_process() {
    let mut app = app();
    let (launch, _rx) = start(&mut app, "worker", "codex", &[]);
    assert!(launch.token.starts_with("launch_"));
    assert_eq!(
        current(&mut app, &launch),
        None,
        "admitted, not yet claimed"
    );

    let claimed = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok", "{claimed}");
    assert_eq!(
        current(&mut app, &launch),
        None,
        "claimed, but not yet the terminal's agent process"
    );

    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(current(&mut app, &launch), Some(launch.token.clone()));

    // Every AgentInfo reports it: the list and the launcher lookup too.
    let list = request(
        &mut app,
        serde_json::json!({"id": "list", "method": "agent.list", "params": {}}),
    );
    assert_eq!(
        list["result"]["agents"][0]["launch_incarnation"],
        launch.token.as_str()
    );
}

#[tokio::test]
async fn the_nonce_is_spent_by_its_first_use() {
    let mut app = app();
    let (launch, _rx) = start(&mut app, "worker", "codex", &[]);
    let first = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(first["result"]["type"], "ok");
    // A line repeated from shell history claims the same nonce again.
    let repeated = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4300, 9), SHELL)),
    );
    assert_eq!(repeated["error"]["code"], "launch_attestation_refused");
    let unknown = claim(
        &mut app,
        &launch,
        "nonce_unknown",
        Some((process(4300, 9), SHELL)),
    );
    assert_eq!(unknown["error"]["code"], "launch_attestation_refused");
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(current(&mut app, &launch), Some(launch.token.clone()));
}

#[tokio::test]
async fn a_claim_that_proves_nothing_ends_the_launch() {
    let refused: [Option<(ProcessInstance, u32)>; 3] = [
        // Not the pane shell's own child.
        Some((process(4242, 7), SHELL + 1)),
        // No start time, so no birth identity.
        Some((
            ProcessInstance {
                pid: 4242,
                started: None,
            },
            SHELL,
        )),
        // The server could not read the sender at all.
        None,
    ];
    for sender in refused {
        let mut app = app();
        let (launch, _rx) = start(&mut app, "worker", "codex", &[]);
        let response = claim(&mut app, &launch, &launch.nonce, sender);
        assert_eq!(
            response["error"]["code"], "launch_attestation_refused",
            "{sender:?}: {response}"
        );
        // The nonce is spent: a later, valid claim cannot revive it.
        let later = claim(
            &mut app,
            &launch,
            &launch.nonce,
            Some((process(4242, 7), SHELL)),
        );
        assert_eq!(later["error"]["code"], "launch_attestation_refused");
        observe(&mut app, &launch, Agent::Codex, process(4242, 7));
        assert_eq!(current(&mut app, &launch), None, "{sender:?}");
    }
}

#[tokio::test]
async fn a_process_identified_before_the_claim_is_not_the_admissions_process() {
    let mut app = app();
    let (launch, _rx) = start(&mut app, "worker", "codex", &[]);
    // Another Codex took the pane before the helper claimed the launch.
    observe(&mut app, &launch, Agent::Codex, process(5000, 3));
    let late = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(late["error"]["code"], "launch_attestation_refused");
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(current(&mut app, &launch), None);
}

#[tokio::test]
async fn another_process_exit_respawn_or_kind_revokes_the_token_for_good() {
    // A same-kind replacement before any session report.
    let mut app = app();
    let (launch, _rx) = attested(&mut app, Agent::Codex);
    observe(&mut app, &launch, Agent::Codex, process(4243, 8));
    assert_eq!(held(&app, &launch), None);
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(held(&app, &launch), None, "never attested again");

    // A reused process id: same id, later start.
    let mut app = self::app();
    let (launch, _rx) = attested(&mut app, Agent::Codex);
    observe(&mut app, &launch, Agent::Codex, process(4242, 99));
    assert_eq!(held(&app, &launch), None);

    // An observed exit, even if the same process is seen again after it.
    let mut app = self::app();
    let (launch, _rx) = attested(&mut app, Agent::Codex);
    exit(&mut app, &launch, Agent::Codex);
    assert_eq!(held(&app, &launch), None, "absent once the exit is seen");
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(held(&app, &launch), None);

    // Another agent kind in the pane.
    let mut app = self::app();
    let (launch, _rx) = attested(&mut app, Agent::Codex);
    observe(&mut app, &launch, Agent::Claude, process(4300, 9));
    assert_eq!(held(&app, &launch), None);

    // A respawned pane.
    let mut app = self::app();
    let (launch, _rx) = attested(&mut app, Agent::Codex);
    app.state
        .terminals
        .get_mut(&launch.terminal_id)
        .unwrap()
        .clear_agent_runtime_identity_after_respawn();
    app.state
        .terminals
        .get_mut(&launch.terminal_id)
        .unwrap()
        .set_agent_name("worker".into());
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(held(&app, &launch), None);
}

#[tokio::test]
async fn a_new_start_gets_a_new_token_and_the_old_one_is_refused() {
    let mut app = app();
    let (first, _rx) = attested(&mut app, Agent::Codex);
    exit(&mut app, &first, Agent::Codex);
    app.handle_internal_event(crate::events::AppEvent::StateChanged {
        pane_id: first.pane,
        agent: None,
        state: crate::detect::AgentState::Unknown,
        visible_blocker: false,
        visible_working: false,
        process_exited: false,
        observed_at: std::time::Instant::now(),
    });
    let (second, mut rx) = start(&mut app, "worker", "codex", &[]);
    assert_ne!(second.token, first.token);
    assert_ne!(second.nonce, first.nonce);
    let claimed = claim(
        &mut app,
        &second,
        &second.nonce,
        Some((process(4250, 20), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok");
    observe(&mut app, &second, Agent::Codex, process(4250, 20));
    assert_eq!(current(&mut app, &second), Some(second.token.clone()));

    let stale = send_esc(
        &mut app,
        &second,
        serde_json::json!({"expected_launch_incarnation": first.token}),
    );
    assert_refused(&stale, "agent_launch_mismatch", &mut rx);
    assert!(stale["error"]["message"]
        .as_str()
        .unwrap()
        .contains(&second.token));
    let fresh = send_esc(
        &mut app,
        &second,
        serde_json::json!({"expected_launch_incarnation": second.token}),
    );
    assert_written(&fresh, &mut rx);
}

/// Records the official native session report `session` of the pane's Codex,
/// sent by `reporter`.
fn report_session(
    app: &mut App,
    launch: &Launch,
    session: &str,
    reporter: Option<ProcessInstance>,
) {
    app.handle_internal_event(crate::events::AppEvent::AgentSessionReported {
        pane_id: launch.pane,
        source: "herdr:codex".into(),
        agent_label: "codex".into(),
        seq: None,
        session_ref: crate::agent_resume::AgentSessionRef::id(session),
        session_start_source: Some("startup".into()),
        evidence: match reporter {
            Some(reporter) => crate::terminal::SessionReportEvidence::Reporter(vec![reporter]),
            None => crate::terminal::SessionReportEvidence::Unidentified,
        },
    });
}

#[tokio::test]
async fn send_keys_checks_the_launch_with_the_session_in_the_writing_step() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Codex);
    report_session(&mut app, &launch, "ses_a", Some(process(4242, 7)));

    let both = serde_json::json!({
        "expected_session": "ses_a", "expected_launch_incarnation": launch.token
    });
    assert_written(&send_esc(&mut app, &launch, both.clone()), &mut rx);
    assert_written(&send_esc(&mut app, &launch, serde_json::json!({})), &mut rx);

    let wrong_launch = send_esc(
        &mut app,
        &launch,
        serde_json::json!({"expected_session": "ses_a", "expected_launch_incarnation": "launch_00000000000000000000000000000000"}),
    );
    assert_refused(&wrong_launch, "agent_launch_mismatch", &mut rx);
    let wrong_session = send_esc(
        &mut app,
        &launch,
        serde_json::json!({"expected_session": "ses_b", "expected_launch_incarnation": launch.token}),
    );
    assert_refused(&wrong_session, "agent_session_mismatch", &mut rx);

    // The synchronous dispatcher never writes a launch-guarded send itself.
    let sync = request(
        &mut app,
        serde_json::json!({"id": "esc", "method": "agent.send_keys", "params": {
            "target": launch.public_pane, "keys": ["esc"], "expected_launch_incarnation": launch.token
        }}),
    );
    assert_eq!(sync["error"]["code"], "invalid_request", "{sync}");
    assert!(rx.try_recv().is_err());

    // Replaced between the caller's read and its send, once detection saw it.
    let read = current(&mut app, &launch).unwrap();
    observe(&mut app, &launch, Agent::Codex, process(4243, 8));
    let replaced = send_esc(
        &mut app,
        &launch,
        serde_json::json!({"expected_launch_incarnation": read}),
    );
    assert_refused(&replaced, "agent_launch_mismatch", &mut rx);
    assert!(replaced["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no attested launch"));
}

#[tokio::test]
async fn a_new_launch_resuming_the_same_session_is_not_the_old_launch() {
    let mut app = app();
    let (first, _rx) = attested(&mut app, Agent::Codex);
    report_session(&mut app, &first, "ses_same", Some(process(4242, 7)));
    exit(&mut app, &first, Agent::Codex);
    app.handle_internal_event(crate::events::AppEvent::StateChanged {
        pane_id: first.pane,
        agent: None,
        state: crate::detect::AgentState::Unknown,
        visible_blocker: false,
        visible_working: false,
        process_exited: false,
        observed_at: std::time::Instant::now(),
    });

    let (second, mut rx) = start(&mut app, "worker", "codex", &["resume", "ses_same"]);
    let claimed = claim(
        &mut app,
        &second,
        &second.nonce,
        Some((process(4260, 30), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok");
    observe(&mut app, &second, Agent::Codex, process(4260, 30));
    report_session(&mut app, &second, "ses_same", Some(process(4260, 30)));

    let old = send_esc(
        &mut app,
        &second,
        serde_json::json!({"expected_session": "ses_same", "expected_launch_incarnation": first.token}),
    );
    assert_refused(&old, "agent_launch_mismatch", &mut rx);
    let new = send_esc(
        &mut app,
        &second,
        serde_json::json!({"expected_session": "ses_same", "expected_launch_incarnation": second.token}),
    );
    assert_written(&new, &mut rx);
}

#[tokio::test]
async fn the_guard_parses_strictly_and_refuses_every_noncanonical_form() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Codex);

    // Missing and null are no guard, as for `expected_session`.
    assert_written(&send_esc(&mut app, &launch, serde_json::json!({})), &mut rx);
    assert_written(
        &send_esc(
            &mut app,
            &launch,
            serde_json::json!({"expected_launch_incarnation": null}),
        ),
        &mut rx,
    );

    // Present but not this launch's token, byte for byte.
    for wrong in [
        String::new(),
        " ".into(),
        launch.token.to_uppercase(),
        format!(" {}", launch.token),
        format!("{}\n", launch.token),
        launch.token.trim_start_matches("launch_").to_string(),
        launch.nonce.clone(),
    ] {
        let response = send_esc(
            &mut app,
            &launch,
            serde_json::json!({"expected_launch_incarnation": wrong}),
        );
        assert_refused(&response, "agent_launch_mismatch", &mut rx);
    }

    // Malformed values never reach the handler.
    for malformed in [
        serde_json::json!(42),
        serde_json::json!(true),
        serde_json::json!([launch.token]),
        serde_json::json!({"token": launch.token}),
    ] {
        let raw = serde_json::json!({"id": "esc", "method": "agent.send_keys", "params": {
            "target": launch.public_pane, "keys": ["esc"], "expected_launch_incarnation": malformed
        }});
        assert!(
            serde_json::from_str::<Request>(&raw.to_string()).is_err(),
            "{raw}"
        );
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn the_attestation_request_parses_strictly() {
    for params in [
        serde_json::json!({}),
        serde_json::json!({"nonce": null}),
        serde_json::json!({"nonce": 7}),
    ] {
        let raw =
            serde_json::json!({"id": "attest", "method": "agent.attest_launch", "params": params});
        assert!(
            serde_json::from_str::<Request>(&raw.to_string()).is_err(),
            "{raw}"
        );
    }
    let mut app = app();
    let (launch, _rx) = start(&mut app, "worker", "codex", &[]);
    for wrong in ["", " ", "NONCE_X"] {
        let response = claim(&mut app, &launch, wrong, Some((process(4242, 7), SHELL)));
        assert_eq!(response["error"]["code"], "launch_attestation_refused");
    }
    // Wrong nonces did not spend the real one.
    let claimed = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok");
}

#[tokio::test]
async fn agent_send_keys_refuses_a_launch_guard_when_no_launch_is_attested() {
    let mut app = app();
    let (launch, mut rx) = start(&mut app, "worker", "pi", &[]);
    app.state
        .terminals
        .get_mut(&launch.terminal_id)
        .unwrap()
        .set_detected_state(Some(Agent::Pi), crate::detect::AgentState::Idle);
    let response = send_esc(
        &mut app,
        &launch,
        serde_json::json!({"expected_launch_incarnation": launch.token}),
    );
    assert_refused(&response, "agent_launch_mismatch", &mut rx);
}

fn launcher_start(app: &mut App) -> (Launch, tokio::sync::mpsc::Receiver<Bytes>) {
    let pane = app.state.workspaces[0].tabs[0].root_pane;
    let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane]
        .attached_terminal_id
        .clone();
    let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
    app.terminal_runtimes.insert(terminal_id.clone(), runtime);
    let public_pane = app.public_pane_id(0, pane).unwrap();
    let reply = request(
        app,
        serde_json::json!({"id": "start", "method": "agent.start_with_launcher", "params": {
            "name": "worker", "kind": "codex", "pane_id": public_pane, "args": [],
            "launcher": {"executable": "/bin/sh", "prefix_args": ["-c", "exec \"$0\" \"$@\"", "codex"], "env": {"FIXTURE": "1"}}
        }}),
    );
    assert_eq!(
        reply["result"]["type"], "agent_started_with_launcher",
        "{reply}"
    );
    let token = reply["result"]["launch_incarnation"]
        .as_str()
        .unwrap()
        .to_string();
    let typed = String::from_utf8(rx.try_recv().unwrap().to_vec()).unwrap();
    assert!(typed.contains("/usr/bin/env -- FIXTURE=1 "), "{typed}");
    assert!(typed.contains(" -- /bin/sh -c "), "{typed}");
    let nonce = typed_nonce(&typed);
    (
        Launch {
            pane,
            public_pane,
            terminal_id,
            token,
            nonce,
        },
        rx,
    )
}

fn confirmed(app: &mut App, launch: &Launch) -> serde_json::Value {
    request(
        app,
        serde_json::json!({"id": "lookup", "method": "agent.launcher_info", "params": {"target": launch.public_pane}}),
    )["result"]
        .clone()
}

#[tokio::test]
async fn a_launcher_binding_is_confirmed_only_for_the_launch_whose_process_reported_it() {
    let mut app = app();
    let (launch, _rx) = launcher_start(&mut app);
    let claimed = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok");
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    app.state
        .terminals
        .get_mut(&launch.terminal_id)
        .unwrap()
        .set_detected_state(Some(Agent::Codex), crate::detect::AgentState::Idle);

    // A report the server cannot attribute, and a stale one from another
    // process of the same kind, fill the session but not the launch.
    report_session(&mut app, &launch, "ses_late", None);
    report_session(&mut app, &launch, "ses_late", Some(process(4100, 2)));
    let lookup = confirmed(&mut app, &launch);
    assert_eq!(
        lookup["binding"]["session"]["value"], "ses_late",
        "{lookup}"
    );
    assert!(lookup["binding"]
        .get("session_launch_incarnation")
        .is_none());
    assert_eq!(lookup["confirmed"], false);

    // The launch's own process reports it.
    report_session(&mut app, &launch, "ses_late", Some(process(4242, 7)));
    let lookup = confirmed(&mut app, &launch);
    assert_eq!(
        lookup["binding"]["session_launch_incarnation"],
        launch.token.as_str()
    );
    assert_eq!(lookup["agent"]["launch_incarnation"], launch.token.as_str());
    assert_eq!(lookup["confirmed"], true, "{lookup}");

    // A replacement leads the foreground before detection saw it: the
    // binding is not confirmed and no launch is reported meanwhile.
    foreground(&app, &launch, Some(process(4243, 8)));
    let lookup = confirmed(&mut app, &launch);
    assert_eq!(lookup["confirmed"], false, "{lookup}");
    assert!(
        lookup["agent"].get("launch_incarnation").is_none(),
        "{lookup}"
    );
    // An unreadable foreground proves nothing either.
    foreground(&app, &launch, None);
    assert_eq!(confirmed(&mut app, &launch)["confirmed"], false);
    // The launch's own process leads again, as when a foreground job of its
    // own ended.
    foreground(&app, &launch, Some(process(4242, 7)));
    assert_eq!(confirmed(&mut app, &launch)["confirmed"], true);

    // Once another process holds the terminal the binding is not confirmed,
    // though it keeps the session and its record of the launch.
    observe(&mut app, &launch, Agent::Codex, process(4243, 8));
    let lookup = confirmed(&mut app, &launch);
    assert_eq!(lookup["confirmed"], false, "{lookup}");
    assert_eq!(
        lookup["binding"]["session_launch_incarnation"],
        launch.token.as_str()
    );
}

#[cfg(unix)]
#[test]
fn a_live_handoff_attests_the_carried_launch_only_for_the_same_process() {
    let mut old = crate::terminal::TerminalState::new(
        crate::terminal::TerminalId::alloc(),
        std::path::PathBuf::from("/"),
    );
    let (launch, nonce) =
        crate::terminal::launch_incarnation::LaunchIncarnation::admit(Agent::Codex);
    let token = launch.token().to_string();
    old.admit_launch_incarnation(Some(launch));
    assert_eq!(
        old.handoff_launch_incarnation(),
        None,
        "an unclaimed admission has no process to carry"
    );
    assert!(old.launch_awaits_claim(&nonce));
    old.claim_launch_incarnation(process(4242, 7), SHELL, Some(SHELL))
        .unwrap();
    old.observe_agent_process(Agent::Codex, process(4242, 7));
    assert_eq!(old.launch_incarnation(), Some(token.as_str()));

    // Carried as the handoff runtime state carries it, through JSON.
    let carried: crate::terminal::launch_incarnation::HandoffLaunchIncarnation =
        serde_json::from_value(
            serde_json::to_value(old.handoff_launch_incarnation().unwrap()).unwrap(),
        )
        .unwrap();
    let continued = |carried| {
        let mut next = crate::terminal::TerminalState::new(
            crate::terminal::TerminalId::alloc(),
            std::path::PathBuf::from("/"),
        );
        next.restore_handoff_launch_incarnation(carried);
        next
    };
    let mut same = continued(carried.clone());
    assert_eq!(
        same.launch_incarnation(),
        None,
        "not before the next server sees it"
    );
    same.observe_agent_process(Agent::Codex, process(4242, 7));
    assert_eq!(same.launch_incarnation(), Some(token.as_str()));

    let mut other = continued(carried);
    other.observe_agent_process(Agent::Codex, process(4242, 8));
    assert_eq!(other.launch_incarnation(), None);
    other.observe_agent_process(Agent::Codex, process(4242, 7));
    assert_eq!(other.launch_incarnation(), None);
}

#[tokio::test]
async fn an_official_report_from_the_claimed_process_attests_it_without_a_probe() {
    // An agent under full lifecycle hooks: its report can arrive before any
    // probe identified its process.
    let mut app = app();
    let (launch, _rx) = start(&mut app, "worker", "codex", &[]);
    let claimed = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok");
    // The claimed process leads the foreground, though no probe saw it.
    foreground(&app, &launch, Some(process(4242, 7)));
    app.state
        .terminals
        .get_mut(&launch.terminal_id)
        .unwrap()
        .set_detected_state(Some(Agent::Codex), crate::detect::AgentState::Idle);

    // Not from the claimed process: neither a report nobody can place nor
    // one another Codex sent.
    report_session(&mut app, &launch, "ses_a", None);
    report_session(&mut app, &launch, "ses_a", Some(process(4300, 9)));
    assert_eq!(current(&mut app, &launch), None);

    report_session(&mut app, &launch, "ses_a", Some(process(4242, 7)));
    assert_eq!(current(&mut app, &launch), Some(launch.token.clone()));
}

/// The independent review's reproducer (review/stale-codex-both-guards-probe.py)
/// as a deterministic test: Codex A started, reported its session and was
/// replaced in the pane by Codex B, a process of the same kind, before
/// detection or any report saw B. A send guarded by A's launch and session
/// must not reach B.
#[tokio::test]
async fn a_replacement_detection_has_not_seen_gets_no_keys_guarded_for_the_old_launch() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Codex);
    report_session(&mut app, &launch, "same-session", Some(process(4242, 7)));
    let read = current(&mut app, &launch).expect("the caller reads A's launch");

    // B leads the foreground; nothing the server recorded changed.
    foreground(&app, &launch, Some(process(4243, 8)));
    assert_eq!(held(&app, &launch), Some(launch.token.clone()));
    assert_eq!(current(&mut app, &launch), None, "not reported for B");

    let both = serde_json::json!({
        "expected_session": "same-session", "expected_launch_incarnation": read
    });
    let response = send_esc(&mut app, &launch, both.clone());
    assert_refused(&response, "agent_launch_mismatch", &mut rx);

    // A reused process id: A's id, another start time.
    foreground(&app, &launch, Some(process(4242, 99)));
    assert_refused(
        &send_esc(&mut app, &launch, both.clone()),
        "agent_launch_mismatch",
        &mut rx,
    );
    // No foreground could be read.
    foreground(&app, &launch, None);
    assert_refused(
        &send_esc(&mut app, &launch, both.clone()),
        "agent_launch_mismatch",
        &mut rx,
    );

    // Only A itself leading the foreground receives the guarded keys.
    foreground(&app, &launch, Some(process(4242, 7)));
    assert_written(&send_esc(&mut app, &launch, both), &mut rx);
}

/// The typed line's other branch: the pane's shell resolved the command
/// itself, through a function or an alias, so the helper ends the admission
/// and the shell runs the command. Nothing of that start is ever attested.
#[tokio::test]
async fn a_start_the_shell_resolves_itself_stays_unattested() {
    let mut app = app();
    let (launch, mut rx) = start(&mut app, "worker", "codex", &[]);
    let request: Request = serde_json::from_value(serde_json::json!({
        "id": "attest", "method": "agent.attest_launch",
        "params": {"nonce": launch.nonce, "resolved_by_shell": true}
    }))
    .unwrap();
    let ended: serde_json::Value =
        serde_json::from_str(&app.handle_api_request_after_internal_events_drained(request, None))
            .unwrap();
    assert_eq!(ended["result"]["type"], "ok", "{ended}");

    // The nonce is spent, and the agent the shell runs is not attested.
    let late = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(late["error"]["code"], "launch_attestation_refused");
    observe(&mut app, &launch, Agent::Codex, process(4242, 7));
    assert_eq!(current(&mut app, &launch), None);
    assert_eq!(held(&app, &launch), None);
    let guarded = send_esc(
        &mut app,
        &launch,
        serde_json::json!({"expected_launch_incarnation": launch.token}),
    );
    assert_refused(&guarded, "agent_launch_mismatch", &mut rx);
    assert_written(&send_esc(&mut app, &launch, serde_json::json!({})), &mut rx);
}

/// Prompts through the handler the server dispatches `agent.prompt` to.
fn prompt(
    app: &mut App,
    launch: &Launch,
    guard: Option<&str>,
) -> std::sync::mpsc::Receiver<String> {
    let mut params = serde_json::json!({"target": launch.public_pane, "text": "hello"});
    if let Some(guard) = guard {
        params["expected_launch_incarnation"] = serde_json::json!(guard);
    }
    let json = serde_json::json!({"id": "prompt", "method": "agent.prompt", "params": params});
    let request: Request = serde_json::from_str(&json.to_string()).expect("a valid request");
    let (respond_to, response) = std::sync::mpsc::channel();
    assert!(app.handle_deferred_agent_api_request(request, respond_to));
    response
}

fn answered(response: &std::sync::mpsc::Receiver<String>) -> serde_json::Value {
    serde_json::from_str(
        &response
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the prompt is answered"),
    )
    .unwrap()
}

#[tokio::test]
async fn a_guarded_prompt_is_typed_and_submitted_for_its_own_launch() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Claude);
    foreground(&app, &launch, Some(process(4242, 7)));

    let response = answered(&prompt(&mut app, &launch, Some(&launch.token.clone())));

    assert_eq!(response["result"]["type"], "agent_prompted", "{response}");
    assert!(rx.try_recv().is_ok(), "the text was typed");
    assert!(rx.try_recv().is_ok(), "and submitted");
}

#[tokio::test]
async fn a_prompt_guarded_for_another_launch_types_nothing() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Claude);
    foreground(&app, &launch, Some(process(4242, 7)));

    let other = format!("launch_{}", "0".repeat(32));
    for guard in [
        other.as_str(),
        // Strictly the token: nothing is trimmed or folded.
        &format!(" {}", launch.token),
        &launch.token.to_uppercase(),
        "",
    ] {
        let response = answered(&prompt(&mut app, &launch, Some(guard)));
        assert_eq!(
            response["error"]["code"], "agent_launch_mismatch",
            "{guard:?}: {response}"
        );
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing was typed"),
            "{response}"
        );
    }
    assert!(rx.try_recv().is_err(), "a refusal types nothing");
}

#[tokio::test]
async fn a_prompt_guarded_for_a_launch_that_left_the_foreground_types_nothing() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Claude);
    let token = launch.token.clone();

    // A replacement the shell started leads the foreground, before detection saw it.
    foreground(&app, &launch, Some(process(4243, 8)));
    let response = answered(&prompt(&mut app, &launch, Some(&token)));
    assert_eq!(
        response["error"]["code"], "agent_launch_mismatch",
        "{response}"
    );

    // No foreground could be read: that proves nothing.
    foreground(&app, &launch, None);
    let response = answered(&prompt(&mut app, &launch, Some(&token)));
    assert_eq!(
        response["error"]["code"], "agent_launch_mismatch",
        "{response}"
    );

    // Its process exited: the token is gone for good.
    foreground(&app, &launch, Some(process(4242, 7)));
    exit(&mut app, &launch, Agent::Claude);
    let response = answered(&prompt(&mut app, &launch, Some(&token)));
    assert_eq!(
        response["error"]["code"], "agent_launch_mismatch",
        "{response}"
    );

    assert!(rx.try_recv().is_err(), "a refusal types nothing");
}

#[tokio::test]
async fn a_launch_lost_between_the_text_and_the_enter_gets_no_enter() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Claude);
    foreground(&app, &launch, Some(process(4242, 7)));

    let response = prompt(&mut app, &launch, Some(&launch.token.clone()));
    assert!(rx.recv().await.is_some(), "the text is typed first");
    // During the delay before Enter another process takes the foreground.
    foreground(&app, &launch, Some(process(4243, 8)));

    let response = answered(&response);
    assert_eq!(
        response["error"]["code"], "agent_prompt_interrupted",
        "{response}"
    );
    assert!(rx.try_recv().is_err(), "no Enter was written");
}

#[tokio::test]
async fn an_unguarded_prompt_is_written_as_before() {
    let mut app = app();
    let (launch, mut rx) = attested(&mut app, Agent::Claude);
    // Whoever leads the foreground: no launch was asked for.
    foreground(&app, &launch, Some(process(4243, 8)));

    let response = answered(&prompt(&mut app, &launch, None));

    assert_eq!(response["result"]["type"], "agent_prompted", "{response}");
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_ok());
}

/// What `agent.launch_status` answers for `token`.
fn launch_status(app: &mut App, token: &str) -> serde_json::Value {
    request(
        app,
        serde_json::json!({"id": "status", "method": "agent.launch_status",
                           "params": {"launch_incarnation": token}}),
    )
}

#[tokio::test]
async fn launch_status_finds_an_attested_launch_by_its_token_alone() {
    let mut app = app();
    let (launch, _rx) = attested(&mut app, Agent::Claude);
    foreground(&app, &launch, Some(process(4242, 7)));

    let reply = launch_status(&mut app, &launch.token);

    let result = &reply["result"];
    assert_eq!(result["type"], "agent_launch_status", "{reply}");
    assert_eq!(result["launch_incarnation"], launch.token.as_str());
    assert_eq!(result["state"], "attested");
    assert_eq!(result["terminal_id"], launch.terminal_id.as_str());
    assert_eq!(result["pane_id"], launch.public_pane.as_str());
    assert_eq!(result["agent"]["launch_incarnation"], launch.token.as_str());
    assert_eq!(result["agent"]["agent_status"], "idle");
}

#[tokio::test]
async fn launch_status_follows_the_launch_when_its_name_goes() {
    let mut app = app();
    let (launch, _rx) = attested(&mut app, Agent::Claude);
    foreground(&app, &launch, Some(process(4242, 7)));

    let renamed = request(
        &mut app,
        serde_json::json!({"id": "rename", "method": "agent.rename",
                           "params": {"target": launch.public_pane, "name": "elsewhere"}}),
    );
    assert_eq!(renamed["result"]["agent"]["name"], "elsewhere", "{renamed}");

    let reply = launch_status(&mut app, &launch.token);
    assert_eq!(reply["result"]["state"], "attested", "{reply}");
    assert_eq!(reply["result"]["agent"]["name"], "elsewhere");
}

/// One reply, one observation: when the launch's process stops or starts
/// leading the foreground while the reply is built, its state and the token
/// its agent carries still agree.
#[tokio::test]
async fn launch_status_never_contradicts_its_own_attestation_across_a_foreground_transition() {
    for leaves in [true, false] {
        let mut app = app();
        let (launch, _rx) = attested(&mut app, Agent::Claude);
        let (before, after) = if leaves {
            (process(4242, 7), process(4300, 9))
        } else {
            (process(4300, 9), process(4242, 7))
        };
        foreground(&app, &launch, Some(before));
        crate::pane::PaneRuntime::test_change_foreground_after_reads(1, Some(after));

        let reply = launch_status(&mut app, &launch.token);
        crate::pane::PaneRuntime::test_change_foreground_after_reads(0, None);

        let result = &reply["result"];
        let carries_token = result["agent"]["launch_incarnation"] == launch.token.as_str();
        assert_eq!(
            result["state"] == "attested",
            carries_token,
            "leaving the foreground: {leaves}; {reply}"
        );
        assert_ne!(result["state"], "none", "{reply}");
    }
}

#[tokio::test]
async fn launch_status_says_held_while_a_terminal_keeps_a_launch_it_cannot_attest_now() {
    let mut app = app();
    let (launch, _rx) = start(&mut app, "worker", "claude", &[]);

    // Admitted, before any claim.
    let reply = launch_status(&mut app, &launch.token);
    assert_eq!(reply["result"]["state"], "held", "{reply}");
    assert_eq!(reply["result"]["terminal_id"], launch.terminal_id.as_str());
    assert_eq!(reply["result"]["pane_id"], launch.public_pane.as_str());

    // Claimed, before detection saw the process.
    let claimed = claim(
        &mut app,
        &launch,
        &launch.nonce,
        Some((process(4242, 7), SHELL)),
    );
    assert_eq!(claimed["result"]["type"], "ok", "{claimed}");
    assert_eq!(
        launch_status(&mut app, &launch.token)["result"]["state"],
        "held"
    );

    // Attested, and not leading the foreground: not gone.
    observe(&mut app, &launch, Agent::Claude, process(4242, 7));
    foreground(&app, &launch, Some(process(4300, 9)));
    let reply = launch_status(&mut app, &launch.token);
    assert_eq!(reply["result"]["state"], "held", "{reply}");
    assert_eq!(
        reply["result"]["agent"]["launch_incarnation"],
        serde_json::Value::Null
    );

    foreground(&app, &launch, Some(process(4242, 7)));
    assert_eq!(
        launch_status(&mut app, &launch.token)["result"]["state"],
        "attested"
    );
}

#[tokio::test]
async fn launch_status_says_none_once_no_terminal_holds_the_launch() {
    // Its process exited.
    let mut first = app();
    let (launch, _rx) = attested(&mut first, Agent::Claude);
    exit(&mut first, &launch, Agent::Claude);
    let reply = launch_status(&mut first, &launch.token);
    assert_eq!(reply["result"]["state"], "none", "{reply}");
    assert_eq!(reply["result"]["pane_id"], serde_json::Value::Null);

    // Its pane closed.
    let mut second = app();
    let (launch, _rx) = attested(&mut second, Agent::Claude);
    let closed = request(
        &mut second,
        serde_json::json!({"id": "close", "method": "pane.close",
                           "params": {"pane_id": launch.public_pane}}),
    );
    assert_eq!(closed["result"]["type"], "ok", "{closed}");
    assert_eq!(
        launch_status(&mut second, &launch.token)["result"]["state"],
        "none"
    );

    // A later start took the terminal.
    let mut third = app();
    let (launch, _rx) = attested(&mut third, Agent::Claude);
    let (later, _nonce) =
        crate::terminal::launch_incarnation::LaunchIncarnation::admit(Agent::Claude);
    third
        .state
        .terminals
        .get_mut(&launch.terminal_id)
        .unwrap()
        .admit_launch_incarnation(Some(later));
    assert_eq!(
        launch_status(&mut third, &launch.token)["result"]["state"],
        "none"
    );

    // A token no launch of this server ever had.
    let mut fourth = app();
    let unknown = format!("launch_{}", "0".repeat(32));
    assert_eq!(
        launch_status(&mut fourth, &unknown)["result"]["state"],
        "none"
    );
}

#[tokio::test]
async fn launch_status_refuses_what_is_not_a_token() {
    let mut app = app();
    for value in [
        "",
        "launch_",
        "LAUNCH_00000000000000000000000000000000",
        " launch_00000000000000000000000000000000",
    ] {
        let reply = launch_status(&mut app, value);
        assert_eq!(
            reply["error"]["code"], "invalid_launch_incarnation",
            "{value:?}: {reply}"
        );
    }
}

#[test]
fn a_launch_carried_by_a_live_handoff_is_held_before_the_next_server_attests_it() {
    let mut old = crate::terminal::TerminalState::new(
        crate::terminal::TerminalId::alloc(),
        std::path::PathBuf::from("/"),
    );
    let (launch, _nonce) =
        crate::terminal::launch_incarnation::LaunchIncarnation::admit(Agent::Codex);
    let token = launch.token().to_string();
    old.admit_launch_incarnation(Some(launch));
    old.claim_launch_incarnation(process(4242, 7), SHELL, Some(SHELL))
        .unwrap();
    old.observe_agent_process(Agent::Codex, process(4242, 7));

    let mut next = crate::terminal::TerminalState::new(
        crate::terminal::TerminalId::alloc(),
        std::path::PathBuf::from("/"),
    );
    assert!(!next.holds_launch(&token));
    next.restore_handoff_launch_incarnation(old.handoff_launch_incarnation().unwrap());

    assert_eq!(next.launch_incarnation(), None, "not attested yet");
    assert!(next.holds_launch(&token), "and not gone either");
}
