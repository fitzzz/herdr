use super::*;
use crate::platform::api_readiness_tests::install_poll_hook;
use std::cell::{Cell, RefCell};
use std::io::Read as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::rc::Rc;

thread_local! {
    static CLOCK: RefCell<Option<Rc<Cell<Instant>>>> = const { RefCell::new(None) };
}

pub(super) fn clock_override() -> Option<Instant> {
    CLOCK.with(|slot| {
        slot.borrow().as_ref().map(|clock| {
            let now = clock.get();
            // Advance even if a regression bypasses the poll hook, so it cannot hang a test.
            clock.set(now + Duration::from_millis(1));
            now
        })
    })
}

fn with_clock(test: impl FnOnce(Instant, Rc<Cell<Instant>>)) {
    struct RestoreClock(Option<Rc<Cell<Instant>>>);
    impl Drop for RestoreClock {
        fn drop(&mut self) {
            CLOCK.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let start = Instant::now();
    let clock = Rc::new(Cell::new(start));
    let _restore = RestoreClock(CLOCK.with(|slot| slot.replace(Some(clock.clone()))));
    test(start, clock);
}

fn stream_pair() -> (LocalStream, LocalStream) {
    let (client, server) = UnixStream::pair().unwrap();
    (
        LocalStream::UdSocket(client.into()),
        LocalStream::UdSocket(server.into()),
    )
}

fn assert_blocking(stream: &LocalStream) {
    let LocalStream::UdSocket(socket) = stream;
    // SAFETY: the descriptor is borrowed from the live stream; F_GETFL has no extra argument.
    let flags = unsafe { libc::fcntl(socket.inner().as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(flags & libc::O_NONBLOCK, 0);
}

fn assert_read_poll(descriptor: &libc::pollfd, timeout_ms: i32) {
    assert_eq!(descriptor.events, libc::POLLIN);
    assert_eq!(timeout_ms, 100, "retain the existing finite wait bound");
    // Pending must be a nonblocking observation before the readiness wait.
    // SAFETY: the readiness helper borrows this live descriptor; F_GETFL needs no extra argument.
    let flags = unsafe { libc::fcntl(descriptor.fd, libc::F_GETFL) };
    assert!(flags >= 0);
    assert_ne!(flags & libc::O_NONBLOCK, 0);
}

#[test]
fn initial_request_pending_dispatches_readiness_for_each_fragment() {
    with_clock(|_, _| {
        let (mut client, mut server) = stream_pair();
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let _hook = install_poll_hook(move |descriptor, timeout_ms| {
            assert_read_poll(descriptor, timeout_ms);
            let fragment: &[u8] = match observed.get() {
                0 => b"first",
                1 => b" second\ntrailing",
                _ => panic!("readiness must not spin after the second fragment"),
            };
            observed.set(observed.get() + 1);
            // Bytes arrive only after Pending has dispatched the readiness operation.
            client.write_all(fragment).unwrap();
            None // Delegate to real poll, which must observe the supplied bytes.
        });
        let line =
            read_initial_request_line_with_limits(&mut server, Duration::from_millis(20), 12)
                .unwrap();
        assert_eq!(line.as_deref(), Some("first second\n"));
        assert_eq!(calls.get(), 2);
        assert_blocking(&server);
        let mut trailing = [0; 8];
        server.read_exact(&mut trailing).unwrap();
        assert_eq!(&trailing, b"trailing", "do not read past the first newline");
    });
}

#[test]
fn initial_request_pending_dispatches_readiness_for_eof() {
    for partial in [false, true] {
        with_clock(|_, _| {
            let (client, mut server) = stream_pair();
            let mut client = Some(client);
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let _hook = install_poll_hook(move |descriptor, timeout_ms| {
                assert_read_poll(descriptor, timeout_ms);
                let call = observed.get();
                observed.set(call + 1);
                if partial && call == 0 {
                    client.as_mut().unwrap().write_all(b"partial").unwrap();
                } else {
                    assert_eq!(call, usize::from(partial));
                    drop(client.take());
                }
                None // Real poll must wake for data or the peer's hangup.
            });
            assert_eq!(
                read_initial_request_line_with_limits(&mut server, Duration::from_millis(20), 20)
                    .unwrap(),
                None,
                "EOF discards an unterminated line"
            );
            assert_eq!(calls.get(), 1 + usize::from(partial));
            assert_blocking(&server);
        });
    }
}

#[test]
fn initial_request_quiet_wait_is_finite_without_spinning() {
    with_clock(|start, clock| {
        let (_client, mut server) = stream_pair();
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let timeout = Duration::from_millis(20);
        let _hook = install_poll_hook(move |descriptor, timeout_ms| {
            assert_read_poll(descriptor, timeout_ms);
            assert_eq!(
                observed.replace(1),
                0,
                "one finite wait reaches the deadline"
            );
            clock.set(start + timeout);
            Some(Ok(0)) // Simulate one expired kernel wait, without a real delay.
        });
        let error = read_initial_request_line_with_limits(&mut server, timeout, 20).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "timed out reading api request");
        assert_eq!(calls.get(), 1);
        assert_blocking(&server);
    });
}

#[test]
fn initial_request_partial_progress_does_not_reset_absolute_deadline() {
    with_clock(|start, clock| {
        let (mut client, mut server) = stream_pair();
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let timeout = Duration::from_millis(20);
        let _hook = install_poll_hook(move |descriptor, timeout_ms| {
            assert_read_poll(descriptor, timeout_ms);
            match observed.get() {
                0 => {
                    clock.set(start + timeout - Duration::from_millis(1));
                    client.write_all(b"a").unwrap();
                    observed.set(1);
                    None
                }
                1 => {
                    clock.set(start + timeout);
                    observed.set(2);
                    Some(Ok(0))
                }
                _ => panic!("partial data must not grant another timeout window"),
            }
        });
        let error = read_initial_request_line_with_limits(&mut server, timeout, 20).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(calls.get(), 2);
        assert_blocking(&server);
    });
}

#[test]
fn initial_request_interrupted_wait_rechecks_absolute_deadline() {
    with_clock(|start, clock| {
        let (_client, mut server) = stream_pair();
        let _hook = install_poll_hook(move |descriptor, timeout_ms| {
            assert_read_poll(descriptor, timeout_ms);
            clock.set(start + Duration::from_millis(20));
            Some(Err(io::ErrorKind::Interrupted.into()))
        });
        let error =
            read_initial_request_line_with_limits(&mut server, Duration::from_millis(20), 20)
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_blocking(&server);
    });
}

#[test]
fn initial_request_expired_pending_restores_blocking_without_waiting() {
    with_clock(|_, _| {
        let (_client, mut server) = stream_pair();
        let _hook = install_poll_hook(|_, _| panic!("expired Pending must not wait"));
        let error =
            read_initial_request_line_with_limits(&mut server, Duration::ZERO, 20).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_blocking(&server);
    });
}

#[test]
fn initial_request_wait_errors_restore_blocking() {
    for invalid_descriptor in [false, true] {
        with_clock(|_, _| {
            let (_client, mut server) = stream_pair();
            let _hook = install_poll_hook(move |descriptor, timeout_ms| {
                assert_read_poll(descriptor, timeout_ms);
                if invalid_descriptor {
                    descriptor.revents = libc::POLLNVAL;
                    Some(Ok(1))
                } else {
                    Some(Err(io::Error::from_raw_os_error(libc::EIO)))
                }
            });
            let error =
                read_initial_request_line_with_limits(&mut server, Duration::from_millis(20), 20)
                    .unwrap_err();
            assert_eq!(
                error.raw_os_error(),
                Some(if invalid_descriptor {
                    libc::EBADF
                } else {
                    libc::EIO
                })
            );
            assert_blocking(&server);
        });
    }
}

#[test]
fn initial_request_data_after_deadline_is_still_accepted_without_pending() {
    with_clock(|start, clock| {
        let (mut client, mut server) = stream_pair();
        let _hook = install_poll_hook(move |descriptor, timeout_ms| {
            assert_read_poll(descriptor, timeout_ms);
            clock.set(start + Duration::from_secs(1));
            client.write_all(b"late\n").unwrap();
            None
        });
        assert_eq!(
            read_initial_request_line_with_limits(&mut server, Duration::from_millis(20), 4)
                .unwrap()
                .as_deref(),
            Some("late\n"),
            "the baseline checks the deadline only when a read is Pending"
        );
        assert_blocking(&server);
    });
}

#[test]
fn initial_request_preserves_newline_limit_utf8_and_zero_timeout_boundaries() {
    for (bytes, limit, expected, error_kind) in [
        (b"1234\n".as_slice(), 4, Some("1234\n"), None),
        (
            b"12345\n".as_slice(),
            4,
            None,
            Some(io::ErrorKind::InvalidData),
        ),
        (b"\n".as_slice(), 0, Some("\n"), None),
        (b"x\n".as_slice(), 0, None, Some(io::ErrorKind::InvalidData)),
        (
            b"\xff\n".as_slice(),
            4,
            None,
            Some(io::ErrorKind::InvalidData),
        ),
    ] {
        with_clock(|_, _| {
            let (mut client, mut server) = stream_pair();
            client.write_all(bytes).unwrap();
            let _hook = install_poll_hook(|_, _| panic!("queued bytes must not wait"));
            // Even a zero timeout accepts already queued data, including its newline.
            let result = read_initial_request_line_with_limits(&mut server, Duration::ZERO, limit);
            match error_kind {
                Some(kind) => assert_eq!(result.unwrap_err().kind(), kind),
                None => assert_eq!(result.unwrap().as_deref(), expected),
            }
            assert_blocking(&server);
        });
    }
}
