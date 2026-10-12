use super::*;
use std::cell::RefCell;
use std::io;
use std::os::unix::net::UnixStream;
use std::time::Duration;

type PollHook = Box<dyn FnMut(&mut libc::pollfd, i32) -> Option<io::Result<i32>>>;

thread_local! {
    static POLL_HOOK: RefCell<Option<PollHook>> = const { RefCell::new(None) };
}

pub(crate) struct PollHookGuard(Option<PollHook>);

impl Drop for PollHookGuard {
    fn drop(&mut self) {
        POLL_HOOK.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

pub(crate) fn install_poll_hook(
    hook: impl FnMut(&mut libc::pollfd, i32) -> Option<io::Result<i32>> + 'static,
) -> PollHookGuard {
    PollHookGuard(POLL_HOOK.with(|slot| slot.replace(Some(Box::new(hook)))))
}

pub(super) fn intercept_poll(
    descriptor: &mut libc::pollfd,
    timeout_ms: i32,
) -> Option<io::Result<i32>> {
    POLL_HOOK.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .and_then(|hook| hook(descriptor, timeout_ms))
    })
}

#[test]
fn api_readiness_wait_never_uses_infinite_or_zero_poll_timeout() {
    let (_client, server) = UnixStream::pair().unwrap();
    let server = crate::ipc::LocalStream::UdSocket(server.into());
    for (duration, expected_ms) in [
        (Duration::ZERO, 1),
        (Duration::from_nanos(1), 1),
        (Duration::from_millis(100), 100),
        (Duration::MAX, i32::MAX),
    ] {
        let _hook = install_poll_hook(move |descriptor, timeout_ms| {
            assert_eq!(descriptor.events, libc::POLLIN);
            assert_eq!(timeout_ms, expected_ms);
            Some(Ok(0))
        });
        wait_api_stream_readable(&server, duration).unwrap();
    }
}
