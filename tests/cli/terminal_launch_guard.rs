//! `--expect-launch` on `terminal attach`, `terminal session control` and
//! `terminal session observe`, with real server processes, the real attesting
//! helper, a real agent process in a real pane and, for `terminal attach`, a
//! real PTY around the client.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Child as ProcessChild, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use super::harness::*;

const NEVER_MINTED: &str = "launch_00000000000000000000000000000000";
const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(20);

struct Fixture {
    herdr: SpawnedHerdr,
    base: std::path::PathBuf,
    config_home: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    pane_id: String,
    terminal_id: String,
    token: String,
    lines: std::path::PathBuf,
    pids: std::path::PathBuf,
    held: std::path::PathBuf,
    gate: std::path::PathBuf,
}

impl Fixture {
    /// A server with one started, attested fake `pi` in a pane.
    fn start() -> Self {
        let base = unique_test_dir();
        let config_home = base.join("config");
        let runtime_dir = base.join("runtime");
        let socket_path = runtime_dir.join("herdr.sock");
        let bin = base.join("bin");
        let lines = base.join("pi-lines");
        let pids = base.join("pi-pids");
        let held = base.join("pi-held");
        let gate = base.join("pi-gate");
        fs::create_dir_all(&bin).unwrap();
        let fake_pi = bin.join("pi");
        fs::write(
            &fake_pi,
            format!(
                "#!/bin/sh\necho $$ >> '{pids}'\nexport HERDR_AGENT=pi\n'{herdr}' pane report-agent \"$HERDR_PANE_ID\" --source custom:fake-pi --agent pi --state idle >/dev/null\nwhile IFS= read -r line; do\n  [ \"$line\" = quit ] && exit 0\n  if [ \"$line\" = hold ]; then\n    : > '{held}'\n    while [ ! -e '{gate}' ]; do sleep 0.01; done\n    exit 0\n  fi\n  printf 'line:%s\\n' \"$line\" >> '{lines}'\ndone\n",
                pids = pids.display(),
                herdr = env!("CARGO_BIN_EXE_herdr"),
                lines = lines.display(),
                held = held.display(),
                gate = gate.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&fake_pi, fs::Permissions::from_mode(0o755)).unwrap();

        let herdr = spawn_herdr_with_path(&config_home, &runtime_dir, &socket_path, Some(&bin));
        wait_for_socket(&socket_path, Duration::from_secs(5));
        let created = run_cli_json(
            &socket_path,
            &["workspace", "create", "--cwd", base.to_str().unwrap()],
        );
        let pane_id = created["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_string();
        let mut fixture = Self {
            herdr,
            base,
            config_home,
            socket_path,
            pane_id,
            terminal_id: String::new(),
            token: String::new(),
            lines,
            pids,
            held,
            gate,
        };
        fixture.token = fixture.start_pi("worker");
        fixture
    }

    /// `agent start` in the pane; returns the attested token.
    fn start_pi(&mut self, name: &str) -> String {
        let started = run_cli_json(
            &self.socket_path,
            &[
                "agent",
                "start",
                name,
                "--kind",
                "pi",
                "--pane",
                &self.pane_id,
                "--timeout",
                "8000",
            ],
        );
        assert_eq!(started["result"]["type"], "agent_started", "{started}");
        let token = started["result"]["launch_incarnation"]
            .as_str()
            .expect("the start reply carries the launch incarnation")
            .to_string();
        assert!(
            wait_until(WAIT, POLL, || self.attested().as_deref() == Some(&token)),
            "agent get attests the started launch"
        );
        self.terminal_id = run_cli_json(&self.socket_path, &["agent", "get", &self.pane_id])
            ["result"]["agent"]["terminal_id"]
            .as_str()
            .unwrap()
            .to_string();
        token
    }

    fn attested(&self) -> Option<String> {
        run_cli_json(&self.socket_path, &["agent", "get", &self.pane_id])["result"]["agent"]
            ["launch_incarnation"]
            .as_str()
            .map(str::to_string)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
        command
            .args(args)
            .env("HERDR_SOCKET_PATH", &self.socket_path)
            .env("XDG_CONFIG_HOME", &self.config_home);
        command
    }

    fn type_line(&self, text: &str) {
        for args in [
            ["pane", "send-text", self.pane_id.as_str(), text],
            ["pane", "send-keys", self.pane_id.as_str(), "enter"],
        ] {
            let output = run_cli(&self.socket_path, &args);
            assert!(output.status.success(), "{args:?}: {output:?}");
        }
    }

    fn lines(&self) -> String {
        fs::read_to_string(&self.lines).unwrap_or_default()
    }

    fn started_pis(&self) -> usize {
        fs::read_to_string(&self.pids).map_or(0, |text| text.lines().count())
    }

    /// The launch's process exits and the operator types another agent by
    /// hand into the pane's shell.
    fn replace_by_hand(&self) {
        let before = self.started_pis();
        self.type_line("quit");
        self.type_line("pi");
        assert!(
            wait_until(WAIT, POLL, || self.started_pis() == before + 1),
            "the replacement started"
        );
    }

    fn finish(self) {
        cleanup_spawned_herdr(self.herdr, self.base);
    }
}

/// Asserts one typed refusal: exit status 1, one JSON error on stderr, and
/// no stream on stdout.
fn assert_refused(output: &std::process::Output, code: &str) -> serde_json::Value {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|err| panic!("stderr is one JSON error ({err}): {output:?}"));
    assert_eq!(error["error"]["code"], code, "{error}");
    assert!(output.stdout.is_empty(), "no stream: {output:?}");
    error
}

fn session_args<'a>(mode: &'a str, target: &'a str, token: &'a str) -> Vec<&'a str> {
    vec![
        "terminal",
        "session",
        mode,
        target,
        "--expect-launch",
        token,
    ]
}

/// `terminal attach` inside a real PTY, with its output collected.
struct PtyAttach {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Arc<Mutex<Vec<u8>>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl PtyAttach {
    fn spawn(fixture: &Fixture, args: &[&str]) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_herdr"));
        command.args(args);
        command.env("HERDR_SOCKET_PATH", &fixture.socket_path);
        command.env("XDG_CONFIG_HOME", &fixture.config_home);
        command.env("TERM", "xterm-256color");
        command.env_remove("HERDR_ENV");
        command.env_remove("HERDR_CLIENT_SOCKET_PATH");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let collected = output.clone();
        let reader = thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                collected.lock().unwrap().extend_from_slice(&buffer[..read]);
            }
        });
        let writer = pair.master.take_writer().unwrap();
        Self {
            master: pair.master,
            writer,
            child,
            output,
            reader: Some(reader),
        }
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    fn type_text(&mut self, text: &str) {
        // A write can race the client's exit; the caller checks the effect.
        let _ = self.writer.write_all(text.as_bytes());
        let _ = self.writer.flush();
    }

    /// Waits for the client to exit and for its last output to be collected:
    /// the reader ends once the exit closed the PTY's other side.
    fn wait_for_exit(&mut self) -> u32 {
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                if let Some(reader) = self.reader.take() {
                    reader.join().unwrap();
                }
                return status.exit_code();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the client exits: {}",
                self.output()
            );
            thread::sleep(POLL);
        }
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        drop(self.master);
    }
}

#[test]
fn attach_with_the_right_launch_is_admitted_and_ends_when_that_launch_ends() {
    let fixture = Fixture::start();
    let mut attach = PtyAttach::spawn(
        &fixture,
        &[
            "terminal",
            "attach",
            &fixture.terminal_id,
            "--expect-launch",
            &fixture.token,
        ],
    );
    // The operator's keys reach the launch through the attach.
    assert!(
        wait_until(WAIT, Duration::from_millis(100), || {
            attach.type_text("hello\r");
            fixture.lines().contains("line:hello")
        }),
        "input reached the launch: {}",
        attach.output()
    );

    // The launch's process exits; the attach ends with the reason even though
    // nothing else was typed, and the pane is no longer attached.
    fixture.type_line("quit");
    assert_eq!(attach.wait_for_exit(), 1, "{}", attach.output());
    let output = attach.output();
    assert!(output.contains("terminal_launch_ended"), "{output}");
    assert!(!output.contains("server shut down"), "{output}");
    assert!(output.contains(&fixture.token), "{output}");

    // The terminal can be attached again: the stream released its ownership.
    let free = fixture
        .command(&["terminal", "session", "control", &fixture.terminal_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut free = StreamHelper::new(free);
    assert_eq!(free.next_line()["type"], "terminal.frame");
    free.stop();
    attach.stop();
    fixture.finish();
}

#[test]
fn attach_with_a_wrong_launch_is_refused_with_nothing_attached_or_written() {
    let fixture = Fixture::start();
    let mut attach = PtyAttach::spawn(
        &fixture,
        &[
            "terminal",
            "attach",
            &fixture.terminal_id,
            "--expect-launch",
            NEVER_MINTED,
        ],
    );
    attach.type_text("hello\r");
    assert_eq!(attach.wait_for_exit(), 1, "{}", attach.output());
    let output = attach.output();
    assert!(output.contains("\"terminal_launch_mismatch\""), "{output}");
    assert!(
        output.contains(&fixture.token),
        "names the current launch: {output}"
    );
    // The refusal happened before the client took the terminal over.
    assert!(!output.contains("\u{1b}[?1049h"), "{output:?}");

    // The launch's own input still works, and got nothing from the refused one.
    fixture.type_line("after");
    assert!(wait_until(WAIT, POLL, || fixture
        .lines()
        .contains("line:after")));
    assert_eq!(fixture.lines(), "line:after\n");
    attach.stop();

    // Nobody owns the terminal: a controller needs no takeover.
    let free = fixture
        .command(&["terminal", "session", "control", &fixture.terminal_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut free = StreamHelper::new(free);
    assert_eq!(free.next_line()["type"], "terminal.frame");
    free.stop();
    fixture.finish();
}

#[test]
fn control_and_observe_refuse_a_wrong_launch_and_write_nothing() {
    let fixture = Fixture::start();
    for mode in ["control", "observe"] {
        let mut child = fixture
            .command(&session_args(mode, &fixture.pane_id, NEVER_MINTED))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Input is ready and waiting; a refused control never reads it.
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, r#"{{"type":"terminal.input","text":"leaked\n"}}"#).unwrap();
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert_refused(&output, "terminal_launch_mismatch");
    }
    let cells = fixture
        .command(&[
            "terminal",
            "session",
            "observe",
            &fixture.pane_id,
            "--output",
            "cells",
            "--expect-launch",
            NEVER_MINTED,
        ])
        .output()
        .unwrap();
    assert_refused(&cells, "terminal_launch_mismatch");
    fixture.type_line("after");
    assert!(wait_until(WAIT, POLL, || fixture
        .lines()
        .contains("line:after")));
    assert_eq!(
        fixture.lines(),
        "line:after\n",
        "nothing else reached the launch"
    );
    fixture.finish();
}

#[test]
fn an_absent_occupant_and_a_gone_terminal_are_refused_by_name() {
    let fixture = Fixture::start();
    let created = run_cli_json(
        &fixture.socket_path,
        &[
            "workspace",
            "create",
            "--cwd",
            fixture.base.to_str().unwrap(),
        ],
    );
    let empty_pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_string();
    for mode in ["control", "observe"] {
        let output = fixture
            .command(&session_args(mode, &empty_pane, &fixture.token))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_refused(&output, "terminal_no_occupant");
        let output = fixture
            .command(&session_args(mode, "no-such-terminal", &fixture.token))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_refused(&output, "terminal_not_found");
    }
    let mut attach = PtyAttach::spawn(
        &fixture,
        &[
            "terminal",
            "attach",
            "no-such-terminal",
            "--expect-launch",
            &fixture.token,
        ],
    );
    assert_eq!(attach.wait_for_exit(), 1);
    assert!(
        attach.output().contains("\"terminal_not_found\""),
        "{}",
        attach.output()
    );
    attach.stop();
    fixture.finish();
}

/// The race this guard exists for: a client reads `agent get`, the agent is
/// replaced, and only then does the attach reach the server.
#[test]
fn a_replacement_between_the_read_and_the_attach_is_refused() {
    let mut fixture = Fixture::start();
    let read = fixture.attested().expect("the client reads the launch");
    let (terminal_id, pane_id) = (fixture.terminal_id.clone(), fixture.pane_id.clone());

    // A hand-typed replacement holds no launch at all.
    fixture.replace_by_hand();
    let mut attach = PtyAttach::spawn(
        &fixture,
        &["terminal", "attach", &terminal_id, "--expect-launch", &read],
    );
    attach.type_text("for-the-replacement\r");
    assert_eq!(attach.wait_for_exit(), 1, "{}", attach.output());
    let output = attach.output();
    assert!(
        output.contains("\"terminal_launch_not_attested\"")
            || output.contains("\"terminal_no_occupant\""),
        "{output}"
    );
    attach.stop();
    for mode in ["control", "observe"] {
        let output = fixture
            .command(&session_args(mode, &pane_id, &read))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
    }

    // A later start's launch is a different launch, named in the refusal.
    fixture.type_line("quit");
    assert!(
        wait_until(WAIT, POLL, || {
            let agents = run_cli_json(&fixture.socket_path, &["agent", "list"]);
            agents["result"]["agents"].as_array().is_some_and(|agents| {
                !agents
                    .iter()
                    .any(|a| a["pane_id"] == pane_id.as_str() && a["agent"] == "pi")
            })
        }),
        "the replacement left"
    );
    let second = fixture.start_pi("second");
    assert_ne!(second, read);
    let stale = fixture
        .command(&session_args("observe", &pane_id, &read))
        .output()
        .unwrap();
    let error = assert_refused(&stale, "terminal_launch_mismatch");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&second),
        "{error}"
    );
    assert!(
        !fixture.lines().contains("for-the-replacement"),
        "{}",
        fixture.lines()
    );

    // The current launch is admitted.
    let mut current = StreamHelper::spawn_observe(&fixture, &pane_id, Some(&second));
    assert_eq!(current.next_line()["type"], "terminal.frame");
    current.stop();
    fixture.finish();
}

/// A `terminal session` helper with its JSON lines being read.
struct StreamHelper {
    child: ProcessChild,
    stdout: BufReader<ChildStdout>,
}

impl StreamHelper {
    fn new(mut child: ProcessChild) -> Self {
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdout }
    }

    fn spawn_control(fixture: &Fixture, target: &str, expected: &str) -> Self {
        Self::new(
            fixture
                .command(&session_args("control", target, expected))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }

    fn spawn_observe(fixture: &Fixture, target: &str, expected: Option<&str>) -> Self {
        let mut args = vec!["terminal", "session", "observe", target];
        if let Some(expected) = expected {
            args.extend(["--expect-launch", expected]);
        }
        Self::new(
            fixture
                .command(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }

    fn next_line(&mut self) -> serde_json::Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).unwrap();
        assert_ne!(read, 0, "the stream ended early");
        serde_json::from_str(&line).unwrap()
    }

    /// Reads to the closing record, on another thread so a stream that never
    /// closes fails the test instead of hanging it.
    fn closed_record(mut self) -> (serde_json::Value, Option<i32>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut stdout = self.stdout;
        thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                match stdout.read_line(&mut line) {
                    Ok(read) if read > 0 => {
                        let record: serde_json::Value = serde_json::from_str(&line).unwrap();
                        if record["type"] == "terminal.closed" {
                            let _ = tx.send(record);
                            return;
                        }
                    }
                    _ => return,
                }
            }
        });
        let record = rx
            .recv_timeout(WAIT)
            .expect("the stream closes with a record");
        let status = self.child.wait().unwrap();
        (record, status.code())
    }

    fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Item 3 and item 4 together: on the same replacement a guarded stream ends
/// with the reason and an unguarded one goes on as it always did.
#[test]
fn a_replacement_during_the_stream_ends_only_the_guarded_one() {
    let fixture = Fixture::start();
    let mut guarded = StreamHelper::spawn_observe(&fixture, &fixture.pane_id, Some(&fixture.token));
    assert_eq!(guarded.next_line()["type"], "terminal.frame");
    let mut unguarded = StreamHelper::spawn_observe(&fixture, &fixture.pane_id, None);
    assert_eq!(unguarded.next_line()["type"], "terminal.frame");

    fixture.replace_by_hand();
    let (closed, status) = guarded.closed_record();
    assert_eq!(status, Some(0), "a stream end is not a refusal");
    let reason = closed["reason"].as_str().unwrap();
    assert!(reason.starts_with("terminal_launch_ended: "), "{reason}");
    assert!(reason.contains(&fixture.token), "{reason}");

    assert!(unguarded.is_running(), "the unguarded stream goes on");
    fixture.type_line("still-delivered");
    assert!(wait_until(WAIT, POLL, || fixture
        .lines()
        .contains("line:still-delivered")));
    assert_eq!(unguarded.next_line()["type"], "terminal.frame");
    assert!(unguarded.is_running());
    unguarded.stop();
    fixture.finish();
}

#[test]
fn guarded_control_writes_to_the_launch_until_it_ends_then_stops() {
    let fixture = Fixture::start();
    let mut child = fixture
        .command(&session_args("control", &fixture.pane_id, &fixture.token))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut control = StreamHelper::new(child);
    assert_eq!(control.next_line()["type"], "terminal.frame");
    // Empty input is no write at all, and the stream carries on.
    writeln!(stdin, r#"{{"type":"terminal.input","text":""}}"#).unwrap();
    writeln!(
        stdin,
        r#"{{"type":"terminal.input","text":"via-control\n"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    assert!(wait_until(WAIT, POLL, || fixture
        .lines()
        .contains("line:via-control")));

    fixture.replace_by_hand();
    let (closed, _) = control.closed_record();
    assert!(
        closed["reason"]
            .as_str()
            .unwrap()
            .starts_with("terminal_launch_ended: "),
        "{closed}"
    );
    // The helper exited, so nothing it held can reach the replacement.
    let _ = writeln!(stdin, r#"{{"type":"terminal.input","text":"late\n"}}"#);
    thread::sleep(Duration::from_millis(200));
    assert!(
        !fixture.lines().contains("line:late"),
        "{}",
        fixture.lines()
    );
    fixture.finish();
}

/// Unread bytes waiting in the terminal `pid` reads as its stdin.
fn unread_terminal_input(pid: u32) -> Option<i32> {
    let path = std::ffi::CString::new(format!("/proc/{pid}/fd/0")).unwrap();
    // SAFETY: the path is a valid C string and the descriptor is closed below.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_NOCTTY | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return None;
    }
    let mut unread: libc::c_int = 0;
    // SAFETY: `fd` is open and FIONREAD writes one int to the pointer.
    let result = unsafe { libc::ioctl(fd, libc::FIONREAD as _, &mut unread) };
    // SAFETY: `fd` is open and owned here.
    unsafe { libc::close(fd) };
    (result == 0).then_some(unread)
}

/// The bytes the writer has not reached yet must not outlive the launch they
/// were accepted for. The launch holds still with its terminal full, so one
/// long input is partly written and waits in the writer's queue. The launch
/// then exits and its shell starts a replacement, which reads whatever follows.
#[test]
fn a_queued_stream_write_stops_when_the_launch_is_replaced() {
    const TOTAL: usize = 256 * 1024;
    let fixture = Fixture::start();
    let launch_pid: u32 = fs::read_to_string(&fixture.pids)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut child = fixture
        .command(&session_args("control", &fixture.pane_id, &fixture.token))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut control = StreamHelper::new(child);
    assert_eq!(control.next_line()["type"], "terminal.frame");

    // The replacement's start is already queued behind the line that holds A.
    let typed = run_cli(
        &fixture.socket_path,
        &["pane", "send-text", &fixture.pane_id, "hold\npi\n"],
    );
    assert!(typed.status.success(), "{typed:?}");
    assert!(wait_until(WAIT, POLL, || fixture.held.exists()), "A holds");

    let input = serde_json::json!({"type": "terminal.input", "text": "\n".repeat(TOTAL)});
    writeln!(stdin, "{input}").unwrap();
    stdin.flush().unwrap();
    // The terminal is full while A still leads it: this is a partial write.
    assert!(
        wait_until(WAIT, POLL, || unread_terminal_input(launch_pid)
            .is_some_and(|unread| unread >= 1024)),
        "the input reached the terminal while A led it"
    );
    assert_eq!(fixture.started_pis(), 1, "A is still the only one");
    assert!(fixture.lines().is_empty(), "A read none of it");

    fs::write(&fixture.gate, "").unwrap();
    assert!(
        wait_until(WAIT, POLL, || fixture.started_pis() == 2),
        "the replacement started"
    );
    let (closed, _) = control.closed_record();
    let reason = closed["reason"].as_str().unwrap();
    assert!(reason.starts_with("terminal_launch_ended: "), "{reason}");

    // The replacement reads what was written while A led. That is what the
    // terminal's buffers held, far less than the input; the rest never comes.
    let limit = TOTAL / 4;
    let mut read = 0;
    let mut unchanged_since = Instant::now();
    while unchanged_since.elapsed() < Duration::from_secs(1) {
        let now = fixture.lines().lines().count();
        assert!(now < limit, "the replacement read {now} lines of {TOTAL}");
        if now != read {
            read = now;
            unchanged_since = Instant::now();
        }
        thread::sleep(POLL);
    }
    assert!(read > 0, "the buffered prefix reached the replacement");
    fixture.finish();
}

/// The server takes ownership before it answers. Here the answer to an accepted
/// takeover is lost on the way back, so the client must not report a refusal.
#[test]
fn a_takeover_whose_answer_is_lost_reports_an_unknown_outcome() {
    fn frame(stream: &mut UnixStream) -> Vec<u8> {
        let mut header = [0_u8; 4];
        stream.read_exact(&mut header).unwrap();
        let mut body = vec![0_u8; u32::from_le_bytes(header) as usize];
        stream.read_exact(&mut body).unwrap();
        [header.as_slice(), body.as_slice()].concat()
    }

    let fixture = Fixture::start();
    let mut owner = StreamHelper::spawn_control(&fixture, &fixture.pane_id, &fixture.token);
    assert_eq!(owner.next_line()["type"], "terminal.frame");

    // The client derives its stream socket from the API socket's name, so this
    // name reaches the real API and a stream socket that drops the answer.
    let runtime_dir = fixture.socket_path.parent().unwrap();
    let proxied_api = runtime_dir.join("proxied.sock");
    std::os::unix::fs::symlink(&fixture.socket_path, &proxied_api).unwrap();
    let listener = UnixListener::bind(runtime_dir.join("proxied-client.sock")).unwrap();
    let server_stream = runtime_dir.join("herdr-client.sock");
    let proxy = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut server = UnixStream::connect(server_stream).unwrap();
        server.write_all(&frame(&mut client)).unwrap(); // Hello.
        client.write_all(&frame(&mut server)).unwrap(); // Welcome.
        server.write_all(&frame(&mut client)).unwrap(); // Guarded takeover.
        frame(&mut server) // The answer, dropped as both connections close.
    });

    let second = fixture
        .command(&[
            "terminal",
            "session",
            "control",
            &fixture.pane_id,
            "--takeover",
            "--expect-launch",
            &fixture.token,
        ])
        .env("HERDR_SOCKET_PATH", &proxied_api)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!proxy.join().unwrap().is_empty(), "the server answered");

    let error = assert_refused(&second, "terminal_admission_unanswered");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("may have taken effect"), "{error}");
    // The takeover did happen, which is why this cannot be called a refusal.
    let (closed, _) = owner.closed_record();
    assert_eq!(closed["reason"], "terminal attach taken over", "{closed}");
    fixture.finish();
}

#[test]
fn guarded_cells_observe_starts_its_stream_only_after_admission() {
    let fixture = Fixture::start();
    let mut helper = StreamHelper::new(
        fixture
            .command(&[
                "terminal",
                "session",
                "observe",
                &fixture.pane_id,
                "--output",
                "cells",
                "--expect-launch",
                &fixture.token,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stream = helper.next_line();
    assert_eq!(stream["type"], "terminal.stream", "{stream}");
    assert_eq!(stream["encoding"], "cells", "{stream}");
    helper.stop();
    fixture.finish();
}

#[test]
fn a_malformed_launch_is_refused_before_any_connection() {
    // No server answers here: reaching one would be a different error.
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("absent.sock");
    let malformed = [
        "",
        "launch_",
        "launch_0123456789abcdef0123456789abcde",
        "launch_0123456789abcdef0123456789abcdef0",
        "launch_0123456789ABCDEF0123456789ABCDEF",
        " launch_0123456789abcdef0123456789abcdef",
        "launch_0123456789abcdef0123456789abcdef ",
        "launch_0123456789abcdef0123456789abcdef\n",
        "launch_0123456789abcdef\t123456789abcdef",
        "launch_0123456789abcdef\u{1}123456789abcdef",
        "--takeover",
        "--expect-launch",
        "-h",
    ];
    for prefix in [
        &["terminal", "attach", "term"][..],
        &["terminal", "session", "control", "term"][..],
        &["terminal", "session", "observe", "term"][..],
    ] {
        for value in malformed {
            let mut args = prefix.to_vec();
            args.extend(["--expect-launch", value]);
            let output = run_cli(&socket_path, &args);
            assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
            let error: serde_json::Value = serde_json::from_slice(&output.stderr)
                .unwrap_or_else(|err| panic!("{args:?} ({err}): {output:?}"));
            assert_eq!(
                error["error"]["code"], "invalid_launch_incarnation",
                "{args:?}"
            );
            assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        }

        // The attached form is parsed by the same parser.
        let attached = format!("--expect-launch={}", "launch_");
        let mut args = prefix.to_vec();
        args.push(&attached);
        let output = run_cli(&socket_path, &args);
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "invalid_launch_incarnation");

        // A missing or repeated value is a usage error.
        let mut args = prefix.to_vec();
        args.push("--expect-launch");
        assert_eq!(
            run_cli(&socket_path, &args).status.code(),
            Some(2),
            "{args:?}"
        );
        let mut args = prefix.to_vec();
        args.extend([
            "--expect-launch",
            NEVER_MINTED,
            "--expect-launch",
            NEVER_MINTED,
        ]);
        assert_eq!(
            run_cli(&socket_path, &args).status.code(),
            Some(2),
            "{args:?}"
        );
    }
}

/// An older server would ignore the guarded admission, so the CLI asks first
/// and never opens the stream.
#[test]
fn a_server_without_the_capability_gets_no_guarded_admission() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let client_socket_path = base.join("herdr-client.sock");
    let api = UnixListener::bind(&socket_path).unwrap();
    // A client connection would be accepted here and fail the assertion below.
    let streams = UnixListener::bind(&client_socket_path).unwrap();
    streams.set_nonblocking(true).unwrap();

    thread::spawn(move || {
        // Answers pings with no such capability until the test ends.
        let _ = accept_fake_cli_operation_advertising(&api, &["agent_send_keys_expected_session"]);
    });
    for args in [
        &[
            "terminal",
            "attach",
            "term",
            "--expect-launch",
            NEVER_MINTED,
        ][..],
        &[
            "terminal",
            "session",
            "control",
            "term",
            "--expect-launch",
            NEVER_MINTED,
        ][..],
        &[
            "terminal",
            "session",
            "observe",
            "term",
            "--expect-launch",
            NEVER_MINTED,
        ][..],
    ] {
        let output = run_cli(&socket_path, args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "unsupported_capability", "{error}");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("terminal_attach_expected_launch_incarnation"),
            "{error}"
        );
    }
    assert!(
        streams.accept().is_err(),
        "no stream was opened on the client socket"
    );
    let _ = fs::remove_dir_all(base);
}
