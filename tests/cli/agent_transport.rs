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
