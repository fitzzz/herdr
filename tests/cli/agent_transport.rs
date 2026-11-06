use super::harness::*;

#[test]
fn agent_start_waits_through_unknown_then_rejects_blocked() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        let (mut pane_stream, pane_line) = accept_fake_cli_operation(&listener);
        let pane: serde_json::Value = serde_json::from_str(&pane_line).unwrap();
        assert_eq!(pane["method"], "pane.get");
        assert_eq!(pane["params"]["pane_id"], "w1:p1");
        writeln!(
            pane_stream,
            "{}",
            serde_json::json!({
                "id": pane["id"],
                "result": {
                    "type": "pane_info",
                    "pane": { "terminal_id": "term_1" }
                }
            })
        )
        .unwrap();
        pane_stream.flush().unwrap();

        let (mut start_stream, start_line) = accept_fake_cli_operation(&listener);
        let start: serde_json::Value = serde_json::from_str(&start_line).unwrap();
        assert_eq!(start["method"], "agent.start");
        writeln!(
            start_stream,
            "{}",
            serde_json::json!({
                "id": start["id"],
                "result": {
                    "type": "agent_started",
                    "agent": {
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1",
                        "name": "reviewer"
                    },
                    "argv": ["opencode"]
                }
            })
        )
        .unwrap();
        start_stream.flush().unwrap();

        let (mut get_stream, get_line) = accept_fake_cli_operation(&listener);
        let get: serde_json::Value = serde_json::from_str(&get_line).unwrap();
        assert_eq!(get["method"], "agent.get");
        assert_eq!(get["params"]["target"], "reviewer");
        writeln!(
            get_stream,
            "{}",
            serde_json::json!({
                "id": get["id"],
                "result": {
                    "type": "agent_info",
                    "agent": {
                        "agent": null,
                        "agent_status": "unknown",
                        "interactive_ready": true,
                        "launch_pending": false,
                        "name": "reviewer",
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1"
                    }
                }
            })
        )
        .unwrap();
        get_stream.flush().unwrap();

        let (mut get_stream, get_line) = accept_fake_cli_operation(&listener);
        let get: serde_json::Value = serde_json::from_str(&get_line).unwrap();
        assert_eq!(get["method"], "agent.get");
        assert_eq!(get["params"]["target"], "reviewer");
        writeln!(
            get_stream,
            "{}",
            serde_json::json!({
                "id": get["id"],
                "result": {
                    "type": "agent_info",
                    "agent": {
                        "agent": "opencode",
                        "agent_status": "blocked",
                        "interactive_ready": true,
                        "launch_pending": false,
                        "name": "reviewer",
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1"
                    }
                }
            })
        )
        .unwrap();
        get_stream.flush().unwrap();
    });

    let started = run_cli(
        &socket_path,
        &[
            "agent", "start", "reviewer", "--kind", "opencode", "--pane", "w1:p1",
        ],
    );
    assert_eq!(started.status.code(), Some(1));
    assert!(started.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&started.stderr).unwrap();
    assert_eq!(error["error"]["code"], "agent_not_ready");

    server.join().unwrap();
    cleanup_test_base(&base);
}

#[test]
fn agent_start_does_not_retry_after_the_target_terminal_changes() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, line) = accept_fake_cli_operation(&listener);
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "pane.get");
            writeln!(
                stream,
                "{}",
                serde_json::json!({
                    "id": request["id"],
                    "result": {
                        "type": "pane_info",
                        "pane": { "terminal_id": "term_1" }
                    }
                })
            )
            .unwrap();
            stream.flush().unwrap();

            let (mut stream, line) = accept_fake_cli_operation(&listener);
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            if request["method"] == "agent.start" {
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({
                        "id": request["id"],
                        "error": {
                            "code": "agent_pane_busy",
                            "message": "agent target pane w1:p1 is not an available shell"
                        }
                    })
                )
                .unwrap();
                stream.flush().unwrap();
            } else {
                assert_eq!(request["method"], "pane.process_info");
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({
                        "id": request["id"],
                        "result": {
                            "type": "pane_process_info",
                            "process_info": {
                                "pane_id": "w1:p1",
                                "shell_pid": 10,
                                "foreground_process_group_id": 10,
                                "foreground_processes": [
                                    { "pid": 10, "name": "bash" },
                                    { "pid": 11, "name": "startup-helper" }
                                ]
                            }
                        }
                    })
                )
                .unwrap();
                stream.flush().unwrap();
            }
        }

        let (mut stream, line) = accept_fake_cli_operation(&listener);
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "pane.get");
        writeln!(
            stream,
            "{}",
            serde_json::json!({
                "id": request["id"],
                "result": {
                    "type": "pane_info",
                    "pane": { "terminal_id": "term_2" }
                }
            })
        )
        .unwrap();
        stream.flush().unwrap();
    });

    let started = run_cli(
        &socket_path,
        &[
            "agent", "start", "reviewer", "--kind", "pi", "--pane", "w1:p1",
        ],
    );
    assert_eq!(started.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&started.stderr).unwrap();
    assert_eq!(error["error"]["code"], "agent_pane_busy");

    server.join().unwrap();
    cleanup_test_base(&base);
}

#[test]
fn prompt_wait_is_sent_as_one_agent_request() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        let (mut prompt_stream, prompt_line) = accept_fake_cli_operation(&listener);
        let prompt: serde_json::Value = serde_json::from_str(&prompt_line).unwrap();
        assert_eq!(prompt["method"], "agent.prompt");
        assert_eq!(prompt["params"]["target"], "w1:p1");
        assert_eq!(
            prompt["params"]["wait"]["until"],
            serde_json::json!(["idle"])
        );
        assert!(prompt["params"]["wait"].get("timeout_ms").is_none());
        writeln!(
            prompt_stream,
            "{}",
            serde_json::json!({
                "id": prompt["id"],
                "result": {
                    "type": "agent_prompted",
                    "agent": {
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1",
                        "name": "reviewer",
                        "agent": "pi",
                        "agent_status": "idle",
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "focused": true,
                        "revision": 0
                    }
                }
            })
        )
        .unwrap();
        prompt_stream.flush().unwrap();
    });

    let prompted = run_cli(
        &socket_path,
        &[
            "agent",
            "prompt",
            "w1:p1",
            "review this",
            "--wait",
            "--until",
            "idle",
        ],
    );
    assert!(
        prompted.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&prompted.stderr)
    );
    let prompted: serde_json::Value = serde_json::from_slice(&prompted.stdout).unwrap();
    assert_eq!(prompted["result"]["agent"]["name"], "reviewer");

    server.join().unwrap();
    cleanup_test_base(&base);
}

#[test]
fn send_keys_expect_session_is_sent_to_a_supporting_server_and_its_refusal_is_reported() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        for answer in [
            serde_json::json!({"result": {"type": "ok"}}),
            serde_json::json!({
                "error": {
                    "code": "agent_session_mismatch",
                    "message": "agent reviewer occupies native session ses_new, not the expected session ses_old; no keys were sent"
                }
            }),
        ] {
            let (mut stream, line) = accept_fake_cli_operation_advertising(
                &listener,
                &["agent_send_keys_expected_session"],
            );
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "agent.send_keys");
            assert_eq!(
                request["params"],
                serde_json::json!({
                    "target": "reviewer",
                    "keys": ["esc", "enter"],
                    "expected_session": "ses_old"
                })
            );
            let mut response = answer;
            response["id"] = request["id"].clone();
            writeln!(stream, "{response}").unwrap();
            stream.flush().unwrap();
        }
    });

    let guarded_send = [
        "agent",
        "send-keys",
        "reviewer",
        "esc",
        "--expect-session",
        "ses_old",
        "enter",
    ];
    let accepted = run_cli(&socket_path, &guarded_send);
    assert!(
        accepted.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let sent = run_cli(&socket_path, &guarded_send);
    server.join().unwrap();
    assert_eq!(sent.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&sent.stderr).unwrap();
    assert_eq!(error["error"]["code"], "agent_session_mismatch");

    for args in [
        &["agent", "send-keys", "reviewer", "esc", "--expect-session"][..],
        &[
            "agent",
            "send-keys",
            "reviewer",
            "--expect-session",
            "ses_old",
        ][..],
        &[
            "agent",
            "send-keys",
            "reviewer",
            "esc",
            "--expect-session",
            "a",
            "--expect-session",
            "b",
        ][..],
    ] {
        let rejected = run_cli(&socket_path, args);
        assert_eq!(rejected.status.code(), Some(2), "{args:?}");
    }
    cleanup_test_base(&base);
}

#[test]
fn send_keys_without_expect_session_omits_the_field() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        let (mut stream, line) = accept_fake_cli_operation(&listener);
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            request["params"],
            serde_json::json!({"target": "reviewer", "keys": ["esc"]})
        );
        writeln!(
            stream,
            "{}",
            serde_json::json!({"id": request["id"], "result": {"type": "ok"}})
        )
        .unwrap();
        stream.flush().unwrap();
    });

    let sent = run_cli(&socket_path, &["agent", "send-keys", "reviewer", "esc"]);
    server.join().unwrap();
    assert!(
        sent.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&sent.stderr)
    );
    cleanup_test_base(&base);
}

#[test]
fn send_keys_expect_session_refuses_an_unsupported_server_before_sending_keys() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    for advertised in [None, Some(false)] {
        let base = unique_test_dir();
        fs::create_dir_all(&base).unwrap();
        let socket_path = base.join("herdr.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&done);
        let operations = Arc::new(Mutex::new(Vec::new()));
        let worker_operations = Arc::clone(&operations);
        let server = thread::spawn(move || {
            while !worker_done.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(err) => panic!("accept: {err}"),
                };
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let result = if request["method"] == "ping" {
                    let mut capabilities = serde_json::json!({"live_handoff": false});
                    if let Some(supported) = advertised {
                        capabilities["agent_send_keys_expected_session"] = supported.into();
                    }
                    serde_json::json!({
                        "type": "pong", "version": "older-server",
                        "protocol": CURRENT_PROTOCOL, "capabilities": capabilities
                    })
                } else {
                    // Old servers deserialize target/keys, ignore expected_session,
                    // enqueue the keys and report success.
                    worker_operations.lock().unwrap().push(request.clone());
                    serde_json::json!({"type": "ok"})
                };
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({
                        "id": request["id"], "result": result
                    })
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });

        let output = run_cli(
            &socket_path,
            &[
                "agent",
                "send-keys",
                "reviewer",
                "esc",
                "--expect-session",
                "ses_old",
            ],
        );
        done.store(true, Ordering::Release);
        server.join().unwrap();
        cleanup_test_base(&base);
        assert!(
            operations.lock().unwrap().is_empty(),
            "a server advertising {advertised:?} must receive no mutating request"
        );
        assert_eq!(
            output.status.code(),
            Some(1),
            "unsupported guard must be refused"
        );
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["id"], "cli:agent:send-keys");
        assert_eq!(error["error"]["code"], "unsupported_capability");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("agent_send_keys_expected_session"));
    }
}

/// A synthetic Codex manifest in the server's private config, so these tests
/// exercise Herdr's prompt evidence contract rather than the bundled Codex
/// screen grammar. Status comes from the title; the input box and its
/// evidence follow the `›` prompt-marker regions with synthetic tokens.
fn write_synthetic_codex_manifest(config_home: &Path) {
    let dir = config_home.join(app_dir_name()).join("agent-detection");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("codex.toml"),
        r#"id = "codex"
version = "9999.01.01.1"
min_engine_version = 5

[[rules]]
id = "working"
state = "working"
priority = 200
region = "osc_title"
visible_working = true
contains = ["TEST-WORKING"]

[[rules]]
id = "idle"
state = "idle"
priority = 100
region = "osc_title"
visible_idle = true
contains = ["TEST-IDLE"]

[[composer]]
id = "draft"
state = "draft"
priority = 600
region = "current_prompt_line"
regex = ['(?m)^›[ \t]+[^/\r\n]']
not = [{ contains = ["TEST-EMPTY"] }]

[[composer]]
id = "queued"
state = "queued_evidence"
priority = 100
region = "whole_recent"
requires_submitted_text = true
contains = ["TEST-QUEUED"]

[[composer]]
id = "submitted"
state = "submitted_evidence"
priority = 100
region = "before_current_prompt_marker"
requires_submitted_text = true
regex = ['(?m)^›(?: |$)']
"#,
    )
    .unwrap();
}

/// A stand-in for an agent that steers a prompt sent during a running turn
/// into that turn: once working, a submitted prompt is drawn as a `›` history
/// entry above the empty input line, with no queued-input block.
fn write_fake_steering_codex(bin: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(bin).unwrap();
    let fake_codex = bin.join("codex");
    fs::write(
        &fake_codex,
        r#"#!/bin/sh
stty -echo
title='TEST-IDLE'
status=''
transcript=''
draw() {
  printf '\033]0;%s\007\033[H\033[2J\033[3J' "$title"
  printf 'TEST-HEADER\n\n%b' "$transcript"
  [ -n "$status" ] && printf '%s\n\n' "$status"
  printf '\342\200\272 TEST-EMPTY\n\n  TEST-FOOTER'
}
draw
while IFS= read -r prompt; do
  case "$prompt" in
    "begin turn")
      title='TEST-WORKING'
      status='• TEST-STATUS'
      transcript="$transcript› $prompt\n\n"
      ;;
    "ignore while working") ;;
    *) transcript="$transcript› $prompt\n" ;;
  esac
  draw
done
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_codex, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn prompt_steered_into_a_working_codex_turn_is_prompted() {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");
    let bin = base.join("bin");
    write_fake_steering_codex(&bin);
    write_synthetic_codex_manifest(&config_home);
    let path = PathBuf::from(format!("{}:/usr/bin:/bin", bin.display()));
    let herdr = spawn_herdr_with_path(&config_home, &runtime_dir, &socket_path, Some(&path));
    wait_for_socket(&socket_path, Duration::from_secs(5));
    let created = run_cli_json(
        &socket_path,
        &["workspace", "create", "--cwd", base.to_str().unwrap()],
    );
    let pane_id = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_string();

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "main",
            "--kind",
            "codex",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
        ],
    );
    assert_eq!(started["result"]["agent"]["agent"], "codex");

    let prompt = |text: &str| {
        let output = run_cli(&socket_path, &["agent", "prompt", "main", text]);
        let body = if output.status.success() {
            &output.stdout
        } else {
            &output.stderr
        };
        serde_json::from_slice::<serde_json::Value>(body).unwrap()
    };

    let begun = prompt("begin turn");
    assert_eq!(begun["result"]["type"], "agent_prompted", "{begun}");
    assert!(wait_until(
        Duration::from_secs(5),
        Duration::from_millis(25),
        || run_cli_json(&socket_path, &["agent", "get", "main"])["result"]["agent"]["agent_status"]
            == "working"
    ));

    let steered = prompt("Steer probe: after the sleep finishes, also say STEERED.");
    assert_eq!(steered["result"]["type"], "agent_prompted", "{steered}");
    assert_eq!(steered["result"]["agent"]["agent_status"], "working");

    // Without a new steered entry or queued message, a working Codex prompt
    // still has no acceptance evidence.
    let ignored = prompt("ignore while working");
    assert_eq!(
        ignored["error"]["code"], "agent_prompt_stalled",
        "{ignored}"
    );
    assert!(ignored["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("no acceptance evidence was observed")));

    cleanup_spawned_herdr(herdr, base);
}

/// A Codex stand-in whose input reader can miss wake-ups the way Codex's
/// edge-triggered epoll reader does. Modes:
/// - `lost-wakeup`: ignores every readiness edge until the whole submission is
///   queued, then reads only on a later write, as live Codex did;
/// - `never`: never reads its input;
/// - `no-focus`: like `lost-wakeup`, but never enables focus reporting;
/// - `discard`: reads input at once but never starts a turn.
///
/// Every byte it reads is appended to `<log>`; each submitted line to
/// `<log>.submissions`.
fn write_fake_wakeup_codex(bin: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(bin).unwrap();
    let fake_codex = bin.join("codex");
    fs::write(
        &fake_codex,
        r#"#!/usr/bin/python3 -I
import fcntl, os, select, struct, sys, termios, tty

mode, submit_len, log = sys.argv[1], int(sys.argv[2]), sys.argv[3]
tty.setraw(0)

def draw(title, body):
    os.write(1, ("\x1b]0;%s\x07\x1b[H\x1b[2J\x1b[3JTEST-HEADER\n\n%s"
                 "› TEST-EMPTY\n\n  TEST-FOOTER" % (title, body)).encode())

def queued():
    return struct.unpack("i", fcntl.ioctl(0, termios.FIONREAD, b"\0\0\0\0"))[0]

if mode != "no-focus":
    os.write(1, b"\x1b[?1004h")
draw("TEST-IDLE", "")
poller = select.epoll()
poller.register(0, select.EPOLLIN | select.EPOLLET)
armed = False
while True:
    poller.poll()
    if mode == "never":
        continue
    if mode in ("lost-wakeup", "no-focus") and not armed:
        armed = queued() >= submit_len
        continue
    data = os.read(0, 65536)
    with open(log, "ab") as out:
        out.write(data)
    text = data.replace(b"\x1b[I", b"").replace(b"\x1b[O", b"")
    if mode == "discard" or b"\r" not in text:
        continue
    line = text.split(b"\r")[0].decode()
    with open(log + ".submissions", "a") as out:
        out.write(line + "\n")
    draw("TEST-WORKING", "› %s\n\n• TEST-STATUS\n\n" % line)
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_codex, fs::Permissions::from_mode(0o755)).unwrap();
}

struct WakeupRun {
    response: serde_json::Value,
    read: Vec<u8>,
    submissions: String,
}

/// Prompt an idle fake Codex in `mode` once, without `--wait`, and collect what
/// it read once the prompt answered.
fn prompt_fake_wakeup_codex(mode: &str, prompt: &str) -> WakeupRun {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");
    let bin = base.join("bin");
    let log = base.join("codex-input");
    write_fake_wakeup_codex(&bin);
    write_synthetic_codex_manifest(&config_home);
    let path = PathBuf::from(format!("{}:/usr/bin:/bin", bin.display()));
    let herdr = spawn_herdr_with_path(&config_home, &runtime_dir, &socket_path, Some(&path));
    wait_for_socket(&socket_path, Duration::from_secs(5));
    let created = run_cli_json(
        &socket_path,
        &["workspace", "create", "--cwd", base.to_str().unwrap()],
    );
    let pane_id = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_string();
    // Without bracketed paste the submission is the text plus one Enter byte.
    let submit_len = (prompt.len() + 1).to_string();
    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "main",
            "--kind",
            "codex",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
            "--",
            mode,
            &submit_len,
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(
        started["result"]["agent"]["agent_status"], "idle",
        "{started}"
    );

    let output = run_cli(&socket_path, &["agent", "prompt", "main", prompt]);
    let body = if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    };
    let response = serde_json::from_slice::<serde_json::Value>(body).unwrap();
    // Let a late read land before inspecting what the agent saw.
    thread::sleep(Duration::from_millis(300));
    let run = WakeupRun {
        response,
        read: fs::read(&log).unwrap_or_default(),
        submissions: fs::read_to_string(base.join("codex-input.submissions")).unwrap_or_default(),
    };
    cleanup_spawned_herdr(herdr, base);
    run
}

#[cfg(target_os = "linux")]
#[test]
fn prompt_unread_by_an_idle_codex_is_released_by_one_focus_wakeup() {
    let run = prompt_fake_wakeup_codex("lost-wakeup", "review the diff");
    assert_eq!(
        run.response["result"]["type"], "agent_prompted",
        "{}",
        run.response
    );
    assert_eq!(run.response["result"]["evidence"], "released_after_wakeup");
    assert_eq!(
        run.submissions, "review the diff\n",
        "exactly one submission"
    );
    assert_eq!(
        run.read, b"review the diff\r\x1b[I",
        "the agent saw the prompt once and one focus-in report"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn prompt_never_read_still_stalls_after_only_the_inert_wakeup() {
    let prompt = "never read this";
    let run = prompt_fake_wakeup_codex("never", prompt);
    assert_eq!(
        run.response["error"]["code"], "agent_prompt_stalled",
        "{}",
        run.response
    );
    let message = run.response["error"]["message"].as_str().unwrap();
    // The text, Enter and the three-byte focus-in report: nothing else was written.
    let queued = prompt.len() + 1 + 3;
    assert!(
        message.contains(&format!(
            "{queued} input bytes are still unread in the agent's terminal input queue (queued, not lost) after one focus-in wake-up"
        )),
        "{message}"
    );
    assert!(
        message.contains("no observed working or blocked state"),
        "{message}"
    );
    assert!(run.read.is_empty() && run.submissions.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn prompt_unread_without_focus_reporting_gets_no_wakeup() {
    let prompt = "no focus reports";
    let run = prompt_fake_wakeup_codex("no-focus", prompt);
    assert_eq!(
        run.response["error"]["code"], "agent_prompt_stalled",
        "{}",
        run.response
    );
    let message = run.response["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(
            "{} input bytes are still unread in the agent's terminal input queue (queued, not lost); focus reporting is off, so no wake-up was sent",
            prompt.len() + 1
        )),
        "{message}"
    );
    assert!(run.read.is_empty() && run.submissions.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn prompt_read_but_not_started_gets_no_wakeup() {
    let run = prompt_fake_wakeup_codex("discard", "read and dropped");
    assert_eq!(
        run.response["error"]["code"], "agent_prompt_stalled",
        "{}",
        run.response
    );
    let message = run.response["error"]["message"].as_str().unwrap();
    assert!(!message.contains("input bytes"), "{message}");
    assert!(!message.contains("wake-up"), "{message}");
    assert_eq!(
        run.read, b"read and dropped\r",
        "no focus-in report was written"
    );
}

#[test]
fn send_keys_expect_launch_is_sent_with_the_session_guard_and_its_refusal_is_reported() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let token = "launch_0123456789abcdef0123456789abcdef";

    let server = thread::spawn(move || {
        for answer in [
            serde_json::json!({"result": {"type": "ok"}}),
            serde_json::json!({
                "error": {
                    "code": "agent_launch_mismatch",
                    "message": "agent reviewer runs no attested launch, not the expected launch launch_0123456789abcdef0123456789abcdef; no keys were sent"
                }
            }),
        ] {
            let (mut stream, line) = accept_fake_cli_operation_advertising(
                &listener,
                &[
                    "agent_send_keys_expected_session",
                    "agent_send_keys_expected_launch_incarnation",
                ],
            );
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "agent.send_keys");
            assert_eq!(
                request["params"],
                serde_json::json!({
                    "target": "reviewer",
                    "keys": ["esc"],
                    "expected_session": "ses_old",
                    "expected_launch_incarnation": token
                })
            );
            let mut response = answer;
            response["id"] = request["id"].clone();
            writeln!(stream, "{response}").unwrap();
            stream.flush().unwrap();
        }
    });

    let guarded_send = [
        "agent",
        "send-keys",
        "reviewer",
        "--expect-launch",
        token,
        "esc",
        "--expect-session",
        "ses_old",
    ];
    let accepted = run_cli(&socket_path, &guarded_send);
    assert!(
        accepted.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let refused = run_cli(&socket_path, &guarded_send);
    server.join().unwrap();
    assert_eq!(refused.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&refused.stderr).unwrap();
    assert_eq!(error["error"]["code"], "agent_launch_mismatch");
    cleanup_test_base(&base);
}

#[test]
fn send_keys_expect_launch_refuses_a_server_without_the_launch_guard_before_sending_keys() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    // The launch guard alone, and both guards on a server that has only the
    // session guard: nothing may reach a server that would ignore a field.
    for (flags, advertised) in [
        (vec!["--expect-launch", "launch_x"], None),
        (vec!["--expect-launch", "launch_x"], Some(false)),
        (
            vec!["--expect-launch", "launch_x", "--expect-session", "ses"],
            Some(false),
        ),
    ] {
        let base = unique_test_dir();
        fs::create_dir_all(&base).unwrap();
        let socket_path = base.join("herdr.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&done);
        let operations = Arc::new(Mutex::new(Vec::new()));
        let worker_operations = Arc::clone(&operations);
        let server = thread::spawn(move || {
            while !worker_done.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(err) => panic!("accept: {err}"),
                };
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let result = if request["method"] == "ping" {
                    let mut capabilities = serde_json::json!({
                        "live_handoff": false,
                        "agent_send_keys_expected_session": true
                    });
                    if let Some(supported) = advertised {
                        capabilities["agent_send_keys_expected_launch_incarnation"] =
                            supported.into();
                    }
                    serde_json::json!({
                        "type": "pong", "version": "older-server",
                        "protocol": CURRENT_PROTOCOL, "capabilities": capabilities
                    })
                } else {
                    worker_operations.lock().unwrap().push(request.clone());
                    serde_json::json!({"type": "ok"})
                };
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({"id": request["id"], "result": result})
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });

        let mut args = vec!["agent", "send-keys", "reviewer", "esc"];
        args.extend(flags.iter().copied());
        let output = run_cli(&socket_path, &args);
        done.store(true, Ordering::Release);
        server.join().unwrap();
        cleanup_test_base(&base);
        assert!(
            operations.lock().unwrap().is_empty(),
            "{flags:?} against {advertised:?} must send no mutating request"
        );
        assert_eq!(output.status.code(), Some(1), "{flags:?}");
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "unsupported_capability");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("agent_send_keys_expected_launch_incarnation")
                && !message.contains("agent_send_keys_expected_session"),
            "only the missing guard is named: {message}"
        );
    }
}

/// A server that changes protocol after admitting a start: the protocol
/// refusal still carries the admission the CLI already received.
#[test]
fn agent_start_keeps_its_admission_when_the_protocol_changes_during_readiness() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let token = format!("launch_{}", "1".repeat(32));
    let receipt = serde_json::json!({
        "type": "agent_started",
        "agent": { "pane_id": "w1:p1", "terminal_id": "term_1", "name": "worker" },
        "argv": ["codex"],
        "launch_incarnation": token,
    });

    let server = {
        let receipt = receipt.clone();
        thread::spawn(move || {
            let (mut pane_stream, pane_line) = accept_fake_cli_operation(&listener);
            let pane: serde_json::Value = serde_json::from_str(&pane_line).unwrap();
            assert_eq!(pane["method"], "pane.get");
            writeln!(
                pane_stream,
                "{}",
                serde_json::json!({
                    "id": pane["id"],
                    "result": { "type": "pane_info", "pane": { "terminal_id": "term_1" } }
                })
            )
            .unwrap();
            let (mut start_stream, start_line) = accept_fake_cli_operation(&listener);
            let start: serde_json::Value = serde_json::from_str(&start_line).unwrap();
            assert_eq!(start["method"], "agent.start");
            writeln!(
                start_stream,
                "{}",
                serde_json::json!({ "id": start["id"], "result": receipt })
            )
            .unwrap();
            // The readiness lookup first checks the protocol, which changed.
            let (mut ping_stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(ping_stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let ping: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(ping["method"], "ping");
            write_fake_pong_advertising(
                &mut ping_stream,
                &ping,
                "newer-build",
                CURRENT_PROTOCOL + 1,
                &[],
            );
        })
    };

    let started = run_cli(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "codex",
            "--pane",
            "w1:p1",
            "--timeout",
            "5000",
        ],
    );
    assert_eq!(started.status.code(), Some(1));
    assert!(started.stdout.is_empty());
    let stderr = String::from_utf8(started.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "reported exactly once: {stderr}");
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(error["error"]["code"], "protocol_mismatch", "{error}");
    let admission = &error["error"]["admission"];
    assert_eq!(admission["status"], "admitted_readiness_unresolved");
    assert_eq!(admission["launch_incarnation"], token.as_str());
    assert_eq!(admission["agent"], receipt["agent"]);
    assert_eq!(admission["argv"], receipt["argv"]);

    server.join().unwrap();
    cleanup_test_base(&base);
}
