use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::api::schema::{
    Method, ResponseResult, TerminalInputOpenParams, TerminalInputReconcileParams,
    TerminalInputStreamInfo, TerminalInputWriteAck, TerminalInputWriteParams,
};
use crate::app::App;
use crate::terminal::input_streams::{
    input_digest, literal_input_digest, InputStreamError, PendingCompletion, StreamPosition,
    WriteClass, WriteWaiter,
};

use super::responses::{encode_error, encode_success};

fn terminal_not_found(id: String, terminal_id: &str) -> String {
    encode_error(
        id,
        "terminal_not_found",
        format!("terminal {terminal_id} not found"),
    )
}

fn stream_error(id: String, err: &InputStreamError) -> String {
    encode_error(id, err.code(), err.message())
}

fn stream_info(terminal_id: &str, incarnation: &str, position: StreamPosition) -> ResponseResult {
    ResponseResult::TerminalInputStream {
        stream: TerminalInputStreamInfo {
            terminal_id: terminal_id.to_owned(),
            incarnation: incarnation.to_owned(),
            stream_id: position.stream_id,
            last_admitted_seq: position.last_admitted_seq,
            last_written_seq: position.last_written_seq,
        },
    }
}

impl App {
    fn terminal_input_runtime(
        &self,
        terminal_id: &str,
    ) -> Option<&crate::terminal::TerminalRuntime> {
        let (terminal_id, _) = self.state.terminals.get_key_value(terminal_id)?;
        self.terminal_runtimes.get(terminal_id)
    }

    pub(super) fn handle_terminal_input_open(
        &mut self,
        id: String,
        params: TerminalInputOpenParams,
    ) -> String {
        let Some(runtime) = self.terminal_input_runtime(&params.terminal_id) else {
            return terminal_not_found(id, &params.terminal_id);
        };
        let mut table = runtime.input_streams().lock();
        let opened = table
            .check_incarnation(params.incarnation.as_deref())
            .and_then(|()| table.open(Instant::now()));
        match opened {
            Ok(position) => encode_success(
                id,
                stream_info(&params.terminal_id, table.incarnation(), position),
            ),
            Err(err) => stream_error(id, &err),
        }
    }

    pub(super) fn handle_terminal_input_reconcile(
        &mut self,
        id: String,
        params: TerminalInputReconcileParams,
    ) -> String {
        let Some(runtime) = self.terminal_input_runtime(&params.terminal_id) else {
            return terminal_not_found(id, &params.terminal_id);
        };
        let mut table = runtime.input_streams().lock();
        let position = table
            .check_incarnation(Some(&params.incarnation))
            .and_then(|()| table.position(&params.stream_id, Instant::now()));
        match position {
            Ok(position) => encode_success(
                id,
                stream_info(&params.terminal_id, table.incarnation(), position),
            ),
            Err(err) => stream_error(id, &err),
        }
    }

    /// Handles acknowledged encoded or literal writes, replying once the
    /// write reached the PTY. Returns false for any other method.
    ///
    /// Admission, de-duplication and queueing happen under the terminal's
    /// stream lock on the application thread, so concurrent requests for one
    /// terminal are totally ordered and the queued byte order matches the
    /// admitted sequence order.
    pub(crate) fn handle_deferred_terminal_input_request(
        &mut self,
        request: crate::api::schema::Request,
        respond_to: std::sync::mpsc::Sender<String>,
    ) -> bool {
        // Both payload forms use one admission and completion path. Keep the
        // published encoded request unchanged and normalize only internally.
        let (params, literal) = match request.method {
            Method::TerminalInputWrite(params) => (params, None),
            Method::TerminalInputWriteLiteral(params) => (
                TerminalInputWriteParams {
                    terminal_id: params.terminal_id,
                    incarnation: params.incarnation,
                    stream_id: params.stream_id,
                    seq: params.seq,
                    text: String::new(),
                    keys: Vec::new(),
                },
                Some(params.literal),
            ),
            _ => return false,
        };
        let id = request.id;
        if literal.as_ref().map_or_else(
            || params.text.is_empty() && params.keys.is_empty(),
            String::is_empty,
        ) {
            let _ = respond_to.send(encode_error(
                id,
                "invalid_params",
                "terminal input write needs a nonempty payload",
            ));
            return true;
        }
        if params.seq == 0 {
            let _ = respond_to.send(encode_error(
                id,
                "invalid_params",
                "sequence numbers start at 1",
            ));
            return true;
        }
        let Some(runtime) = self.terminal_input_runtime(&params.terminal_id) else {
            let _ = respond_to.send(terminal_not_found(id, &params.terminal_id));
            return true;
        };

        let digest = match &literal {
            Some(value) => literal_input_digest(value),
            None => input_digest(&params.text, &params.keys),
        };
        let streams = runtime.input_streams().clone();
        let mut table = streams.lock();
        if let Err(err) = table.check_incarnation(Some(&params.incarnation)) {
            let _ = respond_to.send(stream_error(id, &err));
            return true;
        }
        let waiter = |duplicate: bool| -> WriteWaiter {
            let respond_to = respond_to.clone();
            let id = id.clone();
            let ack = TerminalInputWriteAck {
                terminal_id: params.terminal_id.clone(),
                incarnation: params.incarnation.clone(),
                stream_id: params.stream_id.clone(),
                seq: params.seq,
                digest: digest.hex.clone(),
                duplicate,
            };
            Box::new(move |result: Result<(), String>| {
                let response = match result {
                    Ok(()) => {
                        encode_success(id, ResponseResult::TerminalInputWritten { write: ack })
                    }
                    Err(message) => {
                        let err = InputStreamError::WriteFailed(message);
                        stream_error(id, &err)
                    }
                };
                let _ = respond_to.send(response);
            })
        };

        match table.classify_write(&params.stream_id, params.seq, &digest.key, Instant::now()) {
            Err(err) => {
                let _ = respond_to.send(stream_error(id, &err));
            }
            Ok(WriteClass::DuplicateWritten) => waiter(true)(Ok(())),
            Ok(WriteClass::DuplicatePending) => {
                table.wait_for(&params.stream_id, params.seq, waiter(true));
            }
            Ok(WriteClass::Next) => {
                let encoded = match &literal {
                    Some(value) => Ok(value.as_bytes().to_vec()),
                    None => crate::app::api_helpers::encode_api_input(
                        runtime,
                        &params.text,
                        &params.keys,
                    ),
                };
                let bytes = match encoded {
                    Ok(bytes) => bytes,
                    Err(key) => {
                        let _ = respond_to.send(encode_error(
                            id,
                            "invalid_key",
                            format!("unsupported key {key}"),
                        ));
                        return true;
                    }
                };
                let completions = match streams.completion_sender() {
                    Ok(completions) => completions,
                    Err(err) => {
                        let _ = respond_to.send(encode_error(
                            id,
                            "terminal_unavailable",
                            format!("cannot track terminal input: {err}"),
                        ));
                        return true;
                    }
                };
                // The submission path replies only after its bytes were
                // written to the PTY; an empty Enter part adds nothing.
                let completion = match runtime.queue_user_input_submission(
                    Bytes::from(bytes),
                    Bytes::new(),
                    Duration::ZERO,
                    None,
                ) {
                    Ok(completion) => completion,
                    Err(err) => {
                        let code = if err.kind() == std::io::ErrorKind::WouldBlock {
                            "input_queue_full"
                        } else {
                            "terminal_unavailable"
                        };
                        let _ = respond_to.send(encode_error(id, code, err.to_string()));
                        return true;
                    }
                };
                table.record_admitted(&params.stream_id, params.seq, digest.key, waiter(false));
                drop(table);
                streams.track(
                    &completions,
                    PendingCompletion {
                        stream_id: params.stream_id,
                        seq: params.seq,
                        completion,
                    },
                );
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crate::api::schema::{
        ErrorResponse, Method, Request, ResponseResult, SuccessResponse, TerminalInputOpenParams,
        TerminalInputReconcileParams, TerminalInputStreamInfo, TerminalInputWriteAck,
        TerminalInputWriteLiteralParams, TerminalInputWriteParams,
    };
    use crate::app::App;
    use crate::config::Config;
    use crate::workspace::Workspace;

    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    struct Harness {
        app: App,
        terminals: Vec<String>,
        rxs: Vec<tokio::sync::mpsc::Receiver<bytes::Bytes>>,
    }

    impl Harness {
        fn new(terminal_count: usize) -> Self {
            let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut app = App::new(
                &Config::default(),
                crate::app::AppPolicy::TEST,
                None,
                api_rx,
                crate::api::EventHub::default(),
            );
            app.state.workspaces = (0..terminal_count)
                .map(|index| Workspace::test_new(&format!("input-{index}")))
                .collect();
            app.state.ensure_test_terminals();
            let mut terminals = Vec::new();
            let mut rxs = Vec::new();
            for ws_idx in 0..terminal_count {
                let pane_id = app.state.workspaces[ws_idx].tabs[0].root_pane;
                let terminal_id = app.state.terminal_id_for_pane(ws_idx, pane_id).unwrap();
                let (runtime, rx) =
                    crate::terminal::TerminalRuntime::test_with_channel_capacity(80, 24, 64);
                app.terminal_runtimes.insert(terminal_id.clone(), runtime);
                terminals.push(terminal_id.to_string());
                rxs.push(rx);
            }
            Self {
                app,
                terminals,
                rxs,
            }
        }

        fn runtime(&self, index: usize) -> &crate::terminal::TerminalRuntime {
            self.app
                .terminal_input_runtime(&self.terminals[index])
                .unwrap()
        }

        fn open(
            &mut self,
            index: usize,
            incarnation: Option<&str>,
        ) -> Result<TerminalInputStreamInfo, ErrorResponse> {
            let response = self.app.handle_api_request(Request {
                id: "open".into(),
                method: Method::TerminalInputOpen(TerminalInputOpenParams {
                    terminal_id: self.terminals[index].clone(),
                    incarnation: incarnation.map(str::to_owned),
                }),
            });
            stream_result(&response)
        }

        fn reconcile(
            &mut self,
            stream: &TerminalInputStreamInfo,
        ) -> Result<TerminalInputStreamInfo, ErrorResponse> {
            let response = self.app.handle_api_request(Request {
                id: "reconcile".into(),
                method: Method::TerminalInputReconcile(TerminalInputReconcileParams {
                    terminal_id: stream.terminal_id.clone(),
                    incarnation: stream.incarnation.clone(),
                    stream_id: stream.stream_id.clone(),
                }),
            });
            stream_result(&response)
        }

        /// Sends a write and returns the channel its reply arrives on.
        fn send(
            &mut self,
            stream: &TerminalInputStreamInfo,
            seq: u64,
            text: &str,
            keys: &[&str],
        ) -> mpsc::Receiver<String> {
            let (tx, rx) = mpsc::channel();
            assert!(self.app.handle_deferred_terminal_input_request(
                Request {
                    id: format!("write-{seq}"),
                    method: Method::TerminalInputWrite(TerminalInputWriteParams {
                        terminal_id: stream.terminal_id.clone(),
                        incarnation: stream.incarnation.clone(),
                        stream_id: stream.stream_id.clone(),
                        seq,
                        text: text.into(),
                        keys: keys.iter().map(|key| (*key).to_owned()).collect(),
                    }),
                },
                tx,
            ));
            rx
        }

        fn write(
            &mut self,
            stream: &TerminalInputStreamInfo,
            seq: u64,
            text: &str,
            keys: &[&str],
        ) -> Result<TerminalInputWriteAck, ErrorResponse> {
            let reply = self
                .send(stream, seq, text, keys)
                .recv_timeout(WAIT)
                .unwrap();
            match serde_json::from_str::<SuccessResponse>(&reply) {
                Ok(SuccessResponse {
                    result: ResponseResult::TerminalInputWritten { write },
                    ..
                }) => Ok(write),
                _ => Err(serde_json::from_str(&reply).unwrap()),
            }
        }

        /// Bytes written so far to one terminal, without the empty Enter
        /// part the test channel records for every submission.
        fn send_literal(
            &mut self,
            stream: &TerminalInputStreamInfo,
            seq: u64,
            literal: &str,
        ) -> mpsc::Receiver<String> {
            let (tx, rx) = mpsc::channel();
            assert!(self.app.handle_deferred_terminal_input_request(
                Request {
                    id: format!("literal-{seq}"),
                    method: Method::TerminalInputWriteLiteral(TerminalInputWriteLiteralParams {
                        terminal_id: stream.terminal_id.clone(),
                        incarnation: stream.incarnation.clone(),
                        stream_id: stream.stream_id.clone(),
                        seq,
                        literal: literal.to_owned(),
                    }),
                },
                tx,
            ));
            rx
        }

        fn write_literal(
            &mut self,
            stream: &TerminalInputStreamInfo,
            seq: u64,
            literal: &str,
        ) -> Result<TerminalInputWriteAck, ErrorResponse> {
            let response = self
                .send_literal(stream, seq, literal)
                .recv_timeout(WAIT)
                .unwrap();
            match serde_json::from_str(&response) {
                Ok(SuccessResponse {
                    result: ResponseResult::TerminalInputWritten { write },
                    ..
                }) => Ok(write),
                _ => Err(serde_json::from_str(&response).unwrap()),
            }
        }

        fn written(&mut self, index: usize) -> Vec<u8> {
            let mut bytes = Vec::new();
            while let Ok(chunk) = self.rxs[index].try_recv() {
                bytes.extend_from_slice(&chunk);
            }
            bytes
        }
    }

    fn stream_result(response: &str) -> Result<TerminalInputStreamInfo, ErrorResponse> {
        match serde_json::from_str::<SuccessResponse>(response) {
            Ok(SuccessResponse {
                result: ResponseResult::TerminalInputStream { stream },
                ..
            }) => Ok(stream),
            _ => Err(serde_json::from_str(response).unwrap()),
        }
    }

    fn code(result: Result<impl std::fmt::Debug, ErrorResponse>) -> String {
        result.expect_err("expected an error").error.code
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn writes_are_acknowledged_after_the_pty_write_in_exact_order() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        assert_eq!((stream.last_admitted_seq, stream.last_written_seq), (0, 0));

        for (seq, text) in [(1, "a"), (2, "b"), (3, "c")] {
            let ack = h.write(&stream, seq, text, &[]).unwrap();
            assert_eq!((ack.seq, ack.duplicate), (seq, false));
        }
        assert_eq!(h.written(0), b"abc");
        let position = h.reconcile(&stream).unwrap();
        assert_eq!(
            (position.last_admitted_seq, position.last_written_seq),
            (3, 3)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pipelined_writes_keep_their_sequence_order() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        let replies: Vec<_> = (1..=20)
            .map(|seq| h.send(&stream, seq, &format!("{seq},"), &[]))
            .collect();
        for reply in replies {
            reply.recv_timeout(WAIT).unwrap();
        }
        let expected: String = (1..=20).map(|seq| format!("{seq},")).collect();
        assert_eq!(h.written(0), expected.as_bytes());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loss_before_send_resumes_from_the_reconciled_sequence() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.write(&stream, 1, "ls", &[]).unwrap();
        // Sequence 2 was never sent: the connection dropped first.
        let position = h.reconcile(&stream).unwrap();
        assert_eq!(position.last_written_seq, 1);
        assert_eq!(code(h.write(&stream, 3, " -la", &[])), "sequence_gap");
        h.write(&stream, 2, " -la", &["Enter"]).unwrap();
        assert_eq!(h.written(0), b"ls -la\r");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn applied_write_with_lost_reply_is_reconciled_and_not_written_again() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        drop(h.send(&stream, 1, "make test", &["Enter"]));
        // The client reconnects without knowing whether sequence 1 landed.
        let deadline = std::time::Instant::now() + WAIT;
        let position = loop {
            let position = h.reconcile(&stream).unwrap();
            if position.last_written_seq == 1 || std::time::Instant::now() > deadline {
                break position;
            }
            std::thread::yield_now();
        };
        assert_eq!(position.last_written_seq, 1);
        let ack = h.write(&stream, 1, "make test", &["Enter"]).unwrap();
        assert!(ack.duplicate);
        assert_eq!(h.written(0), b"make test\r");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resent_enter_is_written_once() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        assert!(!h.write(&stream, 1, "", &["Enter"]).unwrap().duplicate);
        assert!(h.write(&stream, 1, "", &["Enter"]).unwrap().duplicate);
        assert!(h.write(&stream, 1, "", &["Enter"]).unwrap().duplicate);
        assert_eq!(h.written(0), b"\r");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_sequence_with_a_different_payload_is_refused() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        let first = h.write(&stream, 1, "rm -i x", &[]).unwrap();
        assert_eq!(
            code(h.write(&stream, 1, "rm -f x", &[])),
            "payload_mismatch"
        );
        assert_eq!(
            code(h.write(&stream, 1, "rm -i x", &["Enter"])),
            "payload_mismatch"
        );
        assert_eq!(h.written(0), b"rm -i x");
        let again = h.write(&stream, 1, "rm -i x", &[]).unwrap();
        assert_eq!(again.digest, first.digest);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unicode_text_is_written_unchanged() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        let text = "héllo wörld 🙂 世界 e\u{301}";
        h.write(&stream, 1, text, &[]).unwrap();
        assert_eq!(h.written(0), text.as_bytes());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paste_honors_live_bracketed_paste_mode() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.write(&stream, 1, "one\ntwo", &[]).unwrap();
        h.runtime(0).test_process_pty_bytes(b"\x1b[?2004h");
        h.write(&stream, 2, "three\nfour", &["Enter"]).unwrap();
        assert_eq!(
            h.written(0),
            b"one\ntwo\x1b[200~three\nfour\x1b[201~\r".to_vec()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn literal_writes_preserve_navigation_graphemes_and_controls_in_paste_mode() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.runtime(0).test_process_pty_bytes(b"\x1b[?2004h");
        let literal = "\x1b[H\x1b[F\x1b[2~\x1b[3~\x1b[5~\x1b[6~e\u{301}👩‍💻\x7f\0";
        h.write_literal(&stream, 1, literal).unwrap();
        assert_eq!(h.written(0), literal.as_bytes());
        h.write(&stream, 2, "paste", &["Enter"]).unwrap();
        assert_eq!(h.written(0), b"\x1b[200~paste\x1b[201~\r");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn literal_and_encoded_writes_share_order_and_reject_cross_kind_replays() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.write(&stream, 1, "before", &[]).unwrap();
        let ack = h.write_literal(&stream, 2, "\x1b[H").unwrap();
        h.write(&stream, 3, "after", &["Enter"]).unwrap();
        assert_eq!(h.written(0), b"before\x1b[Hafter\r");
        assert_eq!(h.reconcile(&stream).unwrap().last_written_seq, 3);
        let replay = h.write_literal(&stream, 2, "\x1b[H").unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.digest, ack.digest);
        assert_eq!(code(h.write(&stream, 2, "\x1b[H", &[])), "payload_mismatch");
        assert_eq!(
            code(h.write_literal(&stream, 1, "before")),
            "payload_mismatch"
        );
        assert_eq!(
            code(h.write_literal(&stream, 2, "\x1b[F")),
            "payload_mismatch"
        );
        assert_eq!(h.written(0), b"");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn literal_lost_reply_reconciles_without_retyping() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        drop(h.send_literal(&stream, 1, "e\u{301}\r"));
        // Wait for the same completion a reconnect would observe, then resend
        // its original sequence rather than allocating another one.
        let ack = h.write_literal(&stream, 1, "e\u{301}\r").unwrap();
        assert!(ack.duplicate);
        assert_eq!(h.reconcile(&stream).unwrap().last_written_seq, 1);
        assert_eq!(h.written(0), "e\u{301}\r".as_bytes());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_literal_writes_leave_the_stream_and_terminal_unchanged() {
        let mut h = Harness::new(2);
        let stream = h.open(0, None).unwrap();
        assert_eq!(code(h.write_literal(&stream, 1, "")), "invalid_params");
        assert_eq!(code(h.write_literal(&stream, 0, "x")), "invalid_params");
        assert_eq!(code(h.write_literal(&stream, 2, "x")), "sequence_gap");
        let other = h.open(1, None).unwrap();
        let crossed = TerminalInputStreamInfo {
            terminal_id: other.terminal_id,
            ..stream.clone()
        };
        assert_eq!(code(h.write_literal(&crossed, 1, "x")), "stale_target");
        assert_eq!(h.reconcile(&stream).unwrap().last_admitted_seq, 0);
        assert_eq!(h.written(0), b"");
        assert_eq!(h.written(1), b"");
        assert!(!h.write_literal(&stream, 1, "x").unwrap().duplicate);
    }

    #[test]
    fn literal_method_rejects_missing_or_mixed_payload_fields() {
        let request = serde_json::json!({"id":"literal", "method":"terminal.input.write_literal", "params":{
            "terminal_id":"term", "incarnation":"inc", "stream_id":"stream", "seq":1, "literal":"x"
        }});
        assert!(serde_json::from_value::<Request>(request.clone()).is_ok());
        for field in ["text", "keys"] {
            let mut mixed = request.clone();
            mixed["params"][field] = if field == "keys" {
                serde_json::json!([])
            } else {
                serde_json::json!("")
            };
            assert!(serde_json::from_value::<Request>(mixed).is_err());
        }
        let mut missing = request;
        missing["params"].as_object_mut().unwrap().remove("literal");
        assert!(serde_json::from_value::<Request>(missing).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn keys_use_the_terminal_live_keyboard_protocol() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.write(&stream, 1, "", &["esc"]).unwrap();
        assert_eq!(h.written(0), b"\x1b");

        h.runtime(0).test_process_pty_bytes(b"\x1b[>1u");
        let expected =
            crate::app::api_helpers::encode_api_input(h.runtime(0), "", &["esc".to_string()])
                .unwrap();
        assert_ne!(expected, b"\x1b", "kitty keyboard mode changes Esc");
        h.write(&stream, 2, "", &["esc"]).unwrap();
        assert_eq!(h.written(0), expected);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_keys_and_empty_payloads_write_nothing_and_keep_the_sequence() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        assert_eq!(code(h.write(&stream, 1, "x", &["prefix+h"])), "invalid_key");
        assert_eq!(code(h.write(&stream, 1, "", &[])), "invalid_params");
        assert_eq!(code(h.write(&stream, 0, "x", &[])), "invalid_params");
        assert_eq!(h.written(0), b"");
        h.write(&stream, 1, "x", &[]).unwrap();
        assert_eq!(h.written(0), b"x");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_clients_keep_independent_streams_on_one_terminal() {
        let mut h = Harness::new(1);
        let a = h.open(0, None).unwrap();
        let b = h.open(0, Some(&a.incarnation)).unwrap();
        assert_ne!(a.stream_id, b.stream_id);
        h.write(&a, 1, "a1", &[]).unwrap();
        h.write(&b, 1, "b1", &[]).unwrap();
        h.write(&a, 2, "a2", &[]).unwrap();
        assert!(h.write(&a, 1, "a1", &[]).unwrap().duplicate);
        assert!(h.write(&b, 1, "b1", &[]).unwrap().duplicate);
        assert_eq!(h.written(0), b"a1b1a2");
        assert_eq!(h.reconcile(&a).unwrap().last_written_seq, 2);
        assert_eq!(h.reconcile(&b).unwrap().last_written_seq, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn streams_and_incarnations_are_bound_to_one_terminal() {
        let mut h = Harness::new(2);
        let first = h.open(0, None).unwrap();
        let second = h.open(1, None).unwrap();
        assert_ne!(first.incarnation, second.incarnation);

        let crossed_incarnation = TerminalInputStreamInfo {
            terminal_id: second.terminal_id.clone(),
            ..first.clone()
        };
        assert_eq!(
            code(h.write(&crossed_incarnation, 1, "x", &[])),
            "stale_target"
        );
        let crossed_stream = TerminalInputStreamInfo {
            stream_id: first.stream_id.clone(),
            ..second.clone()
        };
        assert_eq!(
            code(h.write(&crossed_stream, 1, "x", &[])),
            "stream_not_found"
        );
        h.write(&first, 1, "one", &[]).unwrap();
        h.write(&second, 1, "two", &[]).unwrap();
        assert_eq!(h.written(0), b"one");
        assert_eq!(h.written(1), b"two");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_or_unknown_targets_are_refused() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.write(&stream, 1, "x", &[]).unwrap();

        let unknown_stream = TerminalInputStreamInfo {
            stream_id: "tis_forged".into(),
            ..stream.clone()
        };
        assert_eq!(
            code(h.write(&unknown_stream, 1, "x", &[])),
            "stream_not_found"
        );
        assert_eq!(code(h.reconcile(&unknown_stream)), "stream_not_found");
        assert_eq!(code(h.open(0, Some("inc_forged"))), "stale_target");

        // The same terminal id now names a recreated terminal process.
        let terminal_id = h
            .app
            .state
            .terminals
            .get_key_value(stream.terminal_id.as_str())
            .unwrap()
            .0
            .clone();
        let (replacement, replacement_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_capacity(80, 24, 64);
        h.app.terminal_runtimes.insert(terminal_id, replacement);
        h.rxs[0] = replacement_rx;
        assert_eq!(code(h.write(&stream, 2, "y", &[])), "stale_target");
        assert_eq!(code(h.reconcile(&stream)), "stale_target");
        let rebound = h.open(0, None).unwrap();
        assert_ne!(rebound.incarnation, stream.incarnation);
        assert_eq!(h.written(0), b"");

        let gone = TerminalInputStreamInfo {
            terminal_id: "term_missing".into(),
            ..stream
        };
        assert_eq!(code(h.write(&gone, 2, "y", &[])), "terminal_not_found");
        assert_eq!(code(h.reconcile(&gone)), "terminal_not_found");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_exit_refuses_later_writes_and_fails_writes_in_progress() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        h.write(&stream, 1, "x", &[]).unwrap();

        // The PTY reader side disappears: the queued write cannot complete.
        h.rxs[0].close();
        assert_eq!(code(h.write(&stream, 2, "y", &[])), "write_failed");
        assert_eq!(code(h.write(&stream, 2, "y", &[])), "write_failed");
        assert_eq!(h.reconcile(&stream).unwrap().last_written_seq, 1);

        let terminal_id = h
            .app
            .state
            .terminals
            .get_key_value(stream.terminal_id.as_str())
            .unwrap()
            .0
            .clone();
        h.app.terminal_runtimes.remove(&terminal_id);
        assert_eq!(code(h.write(&stream, 3, "z", &[])), "terminal_not_found");
        assert_eq!(code(h.reconcile(&stream)), "terminal_not_found");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_input_queue_is_not_admitted_and_the_sequence_can_be_retried() {
        let mut h = Harness::new(1);
        let terminal_id = h
            .app
            .state
            .terminals
            .get_key_value(h.terminals[0].as_str())
            .unwrap()
            .0
            .clone();
        let (runtime, rx) = crate::terminal::TerminalRuntime::test_with_channel_capacity(80, 24, 2);
        runtime
            .try_send_bytes(bytes::Bytes::from_static(b"p"))
            .unwrap();
        runtime
            .try_send_bytes(bytes::Bytes::from_static(b"q"))
            .unwrap();
        h.app.terminal_runtimes.insert(terminal_id, runtime);
        h.rxs[0] = rx;
        let stream = h.open(0, None).unwrap();

        assert_eq!(code(h.write(&stream, 1, "x", &[])), "input_queue_full");
        assert_eq!(h.reconcile(&stream).unwrap().last_admitted_seq, 0);
        assert_eq!(h.written(0), b"pq");
        h.write(&stream, 1, "x", &[]).unwrap();
        assert_eq!(h.written(0), b"x");
        assert_eq!(h.reconcile(&stream).unwrap().last_written_seq, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn synchronous_dispatch_points_writes_to_the_deferred_path() {
        let mut h = Harness::new(1);
        let stream = h.open(0, None).unwrap();
        let response = h.app.handle_api_request(Request {
            id: "write".into(),
            method: Method::TerminalInputWrite(TerminalInputWriteParams {
                terminal_id: stream.terminal_id,
                incarnation: stream.incarnation,
                stream_id: stream.stream_id,
                seq: 1,
                text: "x".into(),
                keys: Vec::new(),
            }),
        });
        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "invalid_request");
        assert_eq!(h.written(0), b"");
    }
}
