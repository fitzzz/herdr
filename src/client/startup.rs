use super::*;

/// Runs the thin client and enters the main event loop.
pub fn run_client() -> io::Result<()> {
    run_client_with_mode(None, None, "connecting to server")
}

#[cfg(unix)]
pub fn run_terminal_attach(
    terminal_id: String,
    takeover: bool,
    expected_launch: Option<String>,
) -> io::Result<()> {
    run_client_with_mode(
        Some((terminal_id, takeover, expected_launch)),
        Some(AttachEscapeState::default()),
        "attaching to terminal",
    )
}

#[cfg(windows)]
pub fn run_terminal_attach(
    _terminal_id: String,
    _takeover: bool,
    _expected_launch: Option<String>,
) -> io::Result<()> {
    debug_assert!(!crate::platform::capabilities().direct_terminal_attach);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "direct terminal attach is not supported on Windows yet",
    ))
}
