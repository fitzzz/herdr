//! Platform-specific process and filesystem operations.
//!
//! Centralizes OS-dependent behavior behind a clean boundary so core
//! modules don't scatter `#[cfg]` branches through product logic.

#[cfg(unix)]
pub(crate) mod ssh_agent;

pub(crate) struct HostShutdownMonitor {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl HostShutdownMonitor {
    pub(crate) fn start(
        requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let task = monitor_host_shutdown(requested, wake);
        Self { task }
    }
}

impl Drop for HostShutdownMonitor {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn monitor_host_shutdown(
    _requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _wake: impl Fn() + Send + Sync + 'static,
) -> Option<tokio::task::JoinHandle<()>> {
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub pid: u32,
    pub name: String,
    pub argv0: Option<String>,
    pub argv: Option<Vec<String>>,
    pub cmdline: Option<String>,
    /// When the process started, as [`ProcessInstance::started`].
    pub started: Option<u64>,
}

/// One running process, told apart from a later process given the same id by
/// when it started. `started` is in the platform's own units and compared only
/// on this host; it is `None` where the platform could not read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessInstance {
    pub pid: u32,
    pub started: Option<u64>,
}

impl ProcessInstance {
    /// A process whose start time is unknown, so it matches by id alone.
    #[cfg(test)]
    pub fn unknown_start(pid: u32) -> Self {
        Self { pid, started: None }
    }

    /// Whether `other` is this process: the same id and, when both start
    /// times are known, the same start time.
    pub fn is(&self, other: &Self) -> bool {
        self.pid == other.pid
            && (self.started.is_none() || other.started.is_none() || self.started == other.started)
    }
}

/// A process read while walking up from another, with the id of its parent
/// and its process group, `None` where the platform has no process groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AncestorProcess {
    pub process: ForegroundProcess,
    pub parent: u32,
    pub process_group: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundJob {
    pub process_group_id: u32,
    pub processes: Vec<ForegroundProcess>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Hangup,
    Terminate,
    Kill,
}

/// Why a pane runtime ended, before application persistence policy is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildExitReason {
    Exited,
    Interrupted,
    /// Imported runtimes have no child wait handle in the replacement server.
    #[cfg(unix)]
    Handoff,
    WaitFailed,
}

impl ChildExitReason {
    pub(crate) fn requires_session_checkpoint(self) -> bool {
        match self {
            Self::Interrupted => true,
            #[cfg(unix)]
            Self::Handoff => true,
            _ => false,
        }
    }
}

#[cfg(all(test, unix))]
pub(crate) use unix_common::api_readiness_tests;
#[cfg(unix)]
pub(crate) use unix_common::{
    classify_child_exit, poll_fd_readable, read_fd, shared_ssh_control_path,
    wait_api_stream_readable,
};

#[cfg(not(any(unix, windows)))]
pub(crate) fn classify_child_exit(_status: &portable_pty::ExitStatus) -> ChildExitReason {
    ChildExitReason::Exited
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn launch_executable() -> std::io::Result<std::path::PathBuf> {
    std::env::current_exe()
}

pub(crate) fn detached_custom_command_process(command: &str) -> std::process::Command {
    let mut process = detached_custom_command_process_platform(command);
    configure_background_command(&mut process);
    process
}

pub(crate) fn pane_custom_command_pty_builder(command: &str) -> portable_pty::CommandBuilder {
    pane_custom_command_pty_builder_platform(command)
}

pub(crate) fn apply_pane_runtime_marker(command: &mut portable_pty::CommandBuilder) {
    apply_pane_runtime_marker_platform(command);
}

pub(crate) fn prepare_paste_text_for_pty(text: String) -> String {
    prepare_paste_text_for_pty_platform(text)
}

pub(crate) fn plugin_runtime_path(path: &std::path::Path) -> std::path::PathBuf {
    plugin_runtime_path_platform(path)
}

pub(crate) fn normalize_cwd_for_launch(path: &std::path::Path) -> std::path::PathBuf {
    normalize_cwd_for_launch_platform(path)
}

#[cfg(not(windows))]
fn normalize_cwd_for_launch_platform(path: &std::path::Path) -> std::path::PathBuf {
    path.to_path_buf()
}

#[cfg(not(windows))]
fn plugin_runtime_path_platform(path: &std::path::Path) -> std::path::PathBuf {
    path.to_path_buf()
}

#[cfg(not(windows))]
fn prepare_paste_text_for_pty_platform(text: String) -> String {
    text
}

#[cfg(not(windows))]
pub(crate) fn terminal_title_for_presentation(title: &str) -> &str {
    title
}

#[cfg(not(windows))]
fn apply_pane_runtime_marker_platform(_command: &mut portable_pty::CommandBuilder) {}

pub(crate) fn configure_background_command(command: &mut std::process::Command) {
    configure_background_command_platform(command);
}

#[cfg(not(windows))]
fn configure_background_command_platform(_command: &mut std::process::Command) {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformCapabilities {
    pub(crate) live_handoff: bool,
    pub(crate) direct_terminal_attach: bool,
    pub(crate) preserve_legacy_doubled_escape_input: bool,
}

pub(crate) const fn capabilities() -> PlatformCapabilities {
    PlatformCapabilities {
        live_handoff: cfg!(unix),
        direct_terminal_attach: cfg!(unix),
        preserve_legacy_doubled_escape_input: cfg!(target_os = "macos"),
    }
}

pub(crate) fn terminal_grid_size() -> std::io::Result<(u16, u16)> {
    #[cfg(unix)]
    let (cols, rows) = unix_common::read_terminal_grid_size()?;
    #[cfg(windows)]
    let (cols, rows) = windows::read_terminal_grid_size()?;
    #[cfg(not(any(unix, windows)))]
    let (cols, rows) = fallback::read_terminal_grid_size()?;

    if cols == 0 || rows == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "terminal reported a zero-sized grid",
        ));
    }
    Ok((cols, rows))
}

#[cfg(not(windows))]
pub fn launch_server_daemon_command(command: &mut std::process::Command) -> std::io::Result<u32> {
    command.spawn().map(|child| child.id())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn prepare_server_process(_handoff_import: bool) -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn detach_server_daemon_command(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    #[cfg(target_os = "macos")]
    macos::configure_server_daemon_context(command);

    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn current_process_is_detached_server_daemon() -> bool {
    unsafe { libc::getsid(0) == libc::getpid() }
}

/// Raised by the SIGWINCH handler, consumed by the host resize watcher.
#[cfg(unix)]
static TERMINAL_RESIZE_SIGNALLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn record_terminal_resize_signal(_signal: libc::c_int) {
    TERMINAL_RESIZE_SIGNALLED.store(true, std::sync::atomic::Ordering::Release);
}

/// Records SIGWINCH events that size polling can miss.
#[cfg(unix)]
pub(crate) fn watch_terminal_resize_signal() {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction =
        record_terminal_resize_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // Keep blocking stdin and socket reads from failing with EINTR.
    action.sa_flags = libc::SA_RESTART;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGWINCH, &action, std::ptr::null_mut());
    }
}

#[cfg(not(unix))]
pub(crate) fn watch_terminal_resize_signal() {}

/// Returns whether a terminal size change was signalled since the last call.
#[cfg(unix)]
pub(crate) fn take_terminal_resize_signal() -> bool {
    TERMINAL_RESIZE_SIGNALLED.swap(false, std::sync::atomic::Ordering::AcqRel)
}

/// Windows relies on size polling.
#[cfg(not(unix))]
pub(crate) fn take_terminal_resize_signal() -> bool {
    false
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCommand {
    pub program: &'static str,
    pub args: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LimitedRead {
    Empty,
    Complete(Vec<u8>),
    Oversized,
}

pub(crate) fn read_limited_reader(
    mut reader: impl std::io::Read,
    max_bytes: usize,
) -> std::io::Result<LimitedRead> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];

    while bytes.len() < max_bytes {
        let remaining = max_bytes - bytes.len();
        let read_len = remaining.min(buffer.len());
        let bytes_read = match reader.read(&mut buffer[..read_len]) {
            Ok(bytes_read) => bytes_read,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        if bytes_read == 0 {
            return if bytes.is_empty() {
                Ok(LimitedRead::Empty)
            } else {
                Ok(LimitedRead::Complete(bytes))
            };
        }
        bytes.extend_from_slice(&buffer[..bytes_read]);
    }

    let mut sentinel = [0_u8; 1];
    loop {
        return match reader.read(&mut sentinel) {
            Ok(0) if bytes.is_empty() => Ok(LimitedRead::Empty),
            Ok(0) => Ok(LimitedRead::Complete(bytes)),
            Ok(_) => Ok(LimitedRead::Oversized),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => Err(err),
        };
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteSshConfigPaths {
    pub(crate) user_config: Option<std::path::PathBuf>,
    pub(crate) system_config: Option<std::path::PathBuf>,
    pub(crate) multiplexing: bool,
}

pub(crate) const REMOTE_BRIDGE_IDLE_TIMEOUT_SUPPORTED: bool =
    cfg!(any(target_os = "linux", target_os = "macos"));

#[cfg(unix)]
mod remote_bridge;
#[cfg(all(test, unix))]
mod remote_bridge_tests;
#[cfg(unix)]
mod unix_common;
#[cfg(unix)]
pub(crate) mod unix_image_files;
#[cfg(unix)]
pub(crate) use unix_common::{
    begin_cli_output, end_cli_output, forward_remote_bridge_stdio, RemoteBridgeWake,
};

mod client_state;
pub(crate) use client_state::{create_private_state_file, replace_file, sync_parent_directory};

#[cfg(not(unix))]
pub(crate) fn begin_cli_output() {}

#[cfg(not(unix))]
pub(crate) fn end_cli_output() {}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn available_pane_shell_from_job(child_pid: u32, job: ForegroundJob) -> Option<String> {
    if job.process_group_id != child_pid
        || job.processes.iter().any(|process| process.pid != child_pid)
    {
        return None;
    }
    job.processes
        .into_iter()
        .find(|process| process.pid == child_pid)
        .map(|process| process.name)
        .filter(|name| is_pane_shell_process_name(name))
}

fn normalized_process_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_start_matches('-')
        .trim_end_matches(".exe")
        .to_ascii_lowercase()
}

/// The line a restore types into a pane's shell to resume an agent. On Unix it
/// starts with one space, so bash (`HISTCONTROL=ignorespace`), zsh
/// (`HIST_IGNORE_SPACE`) and fish keep the saved options out of the shell's
/// history; the space counts toward the line the shell must take.
pub(crate) fn interactive_shell_restore_command(
    argv: &[String],
    shell_name: &str,
) -> Option<String> {
    let prefix = if cfg!(windows) { "" } else { " " };
    interactive_shell_command(argv, shell_name).map(|command| format!("{prefix}{command}"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn interactive_unix_shell_command(argv: &[String], shell_name: &str) -> Option<String> {
    let quote: fn(&str) -> String = match normalized_process_name(shell_name).as_str() {
        "pwsh" | "powershell" => quote_powershell_arg,
        "fish" | "xonsh" => quote_fish_arg,
        "csh" | "tcsh" => quote_csh_arg,
        "nu" => quote_nu_arg,
        "elvish" => quote_elvish_arg,
        // Unknown names, including wrappers, keep POSIX quoting.
        _ => quote_posix_arg,
    };
    let mut parts = argv.iter();
    let mut command = quote(parts.next()?);
    for part in parts {
        command.push(' ');
        command.push_str(&quote(part));
    }
    Some(command)
}

/// Inside fish single quotes, and in the Python string literals xonsh reads,
/// `\\` and `\'` are escapes, so every backslash is doubled and every quote
/// escaped.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quote_fish_arg(value: &str) -> String {
    if is_plain_unix_word(value) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// csh and tcsh expand `!` history references even inside single quotes, so a
/// `!` is closed out of the quotes and escaped there, like a quote is.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quote_csh_arg(value: &str) -> String {
    if is_plain_unix_word(value) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''").replace('!', "'\\!'"))
}

/// Nushell single quotes cannot hold a quote, so a value goes in double
/// quotes, where `\\` and `\"` are escapes and `$` is literal.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quote_nu_arg(value: &str) -> String {
    if is_plain_unix_word(value) {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Inside elvish single quotes only a quote is special, and it is doubled.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quote_elvish_arg(value: &str) -> String {
    if is_plain_unix_word(value) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn quote_posix_arg(value: &str) -> String {
    if is_plain_unix_word(value) {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn is_plain_unix_word(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '@' | '%' | '_' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
                )
        })
}

/// Whether the pane's shell can take `command` as one typed line. A shell in
/// this platform's `LINE_EDITING_SHELLS` edits its own line and takes any
/// length; any other shell, a wrapper script included, reads the line in the
/// terminal's canonical mode, which keeps only `CANONICAL_LINE_MAX_BYTES` of
/// it. `shell` is the pane's shell program, a path or a name looked up on
/// `PATH`, resolved through symlinks only when the line is too long for
/// canonical mode.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn interactive_shell_accepts_line(command: &str, shell: &str) -> bool {
    if command.len() <= CANONICAL_LINE_MAX_BYTES {
        return true;
    }
    if LINE_EDITING_SHELLS.is_empty() {
        return false;
    }
    let program = if shell.contains('/') {
        Some(std::path::PathBuf::from(shell))
    } else {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(shell))
                .find(|candidate| candidate.is_file())
        })
    };
    let resolved = program.and_then(|program| std::fs::canonicalize(program).ok());
    let name = resolved
        .as_deref()
        .and_then(std::path::Path::file_name)
        .map_or(
            std::borrow::Cow::Borrowed(shell),
            std::ffi::OsStr::to_string_lossy,
        );
    LINE_EDITING_SHELLS.contains(&normalized_process_name(&name).as_str())
}

pub(crate) fn quote_powershell_arg(value: &str) -> String {
    if !value.is_empty()
        && !value.starts_with('-')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'+' | b'=')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn quote_windows_command_line_arg(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\x0b' | '"'))
    {
        return value.to_string();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
        }
        backslashes = 0;
        quoted.push(ch);
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

pub(crate) fn is_pane_shell_process_name(name: &str) -> bool {
    let normalized = normalized_process_name(name);
    matches!(
        normalized.as_str(),
        "sh" | "bash"
            | "dash"
            | "zsh"
            | "fish"
            | "ksh"
            | "mksh"
            | "csh"
            | "tcsh"
            | "elvish"
            | "xonsh"
            | "nu"
            | "pwsh"
            | "powershell"
            | "cmd"
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process_agent_hint(_pid: u32) -> Option<crate::detect::Agent> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn parse_agent_env_hint(environ: &[u8]) -> Option<crate::detect::Agent> {
    for record in environ.split(|&byte| byte == 0) {
        let Some(value) = record.strip_prefix(b"HERDR_AGENT=") else {
            continue;
        };
        return crate::detect::parse_agent_label(std::str::from_utf8(value).ok()?);
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[derive(Debug)]
pub(crate) struct InputSourceRestore;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn switch_to_ascii_input_source() -> Option<InputSourceRestore> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn pump_input_source_runloop() {}

/// Switches the host keyboard input source while prefix mode is active.
///
/// `App` drives this through a trait so the prefix-mode transitions can be
/// tested with a fake, without touching the real macOS APIs or leaking a
/// platform-specific restore type into `App`.
pub(crate) trait PrefixInputSource {
    /// Switch to an ASCII-capable input source for prefix commands. No-op if
    /// the current source is already ASCII-capable, the platform is
    /// unsupported, or the switch fails. Calling it again before `restore`
    /// keeps the source saved by the first call.
    fn switch_to_ascii(&mut self);

    /// Restore whatever `switch_to_ascii` saved. No-op if nothing was switched.
    fn restore(&mut self);
}

/// Production [`PrefixInputSource`] backed by the per-platform API.
#[derive(Default)]
pub(crate) struct RealPrefixInputSource {
    restore: Option<InputSourceRestore>,
}

impl PrefixInputSource for RealPrefixInputSource {
    fn switch_to_ascii(&mut self) {
        if self.restore.is_none() {
            // Drain pending input-source-change notifications so the read below is fresh (see
            // `pump_input_source_runloop`); a no-op on non-macOS.
            pump_input_source_runloop();
            self.restore = switch_to_ascii_input_source();
        }
    }

    fn restore(&mut self) {
        let _ = self.restore.take();
    }
}

#[cfg(all(test, any(unix, windows)))]
#[test]
fn child_exit_classification_only_checkpoints_interruptions() {
    for code in [0, 1, 130, 255, 0xC0000005] {
        let reason = classify_child_exit(&portable_pty::ExitStatus::with_exit_code(code));
        assert_eq!(reason, ChildExitReason::Exited, "exit code {code:#x}");
        assert!(!reason.requires_session_checkpoint());
    }
    #[cfg(windows)]
    let status = portable_pty::ExitStatus::with_exit_code(0xC000013A);
    #[cfg(not(windows))]
    let status = portable_pty::ExitStatus::with_signal("Terminated: 15");
    assert_eq!(classify_child_exit(&status), ChildExitReason::Interrupted);
    assert!(classify_child_exit(&status).requires_session_checkpoint());
    #[cfg(unix)]
    assert!(ChildExitReason::Handoff.requires_session_checkpoint());
    assert!(!ChildExitReason::WaitFailed.requires_session_checkpoint());
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn terminal_resize_signal_is_recorded_once_per_delivery() {
        watch_terminal_resize_signal();
        assert!(!take_terminal_resize_signal());

        unsafe {
            libc::raise(libc::SIGWINCH);
        }

        assert!(take_terminal_resize_signal());
        assert!(!take_terminal_resize_signal());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn canonical_shell_accepts_line_only_within_its_limit() {
        let dir = std::env::temp_dir().join(format!("herdr-accepts-line-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        let bash = dir.join("bash");
        std::fs::write(&bash, "").expect("placeholder shell should be written");
        let sh = dir.join("sh");
        let wrapper = dir.join("placeholder-shell");
        std::os::unix::fs::symlink(&bash, &sh).expect("sh link should be created");
        std::fs::write(&wrapper, "").expect("placeholder wrapper should be written");
        let to_text = |path: &std::path::Path| path.to_string_lossy().into_owned();

        let at_limit = "a".repeat(CANONICAL_LINE_MAX_BYTES);
        let over_limit = "a".repeat(CANONICAL_LINE_MAX_BYTES + 1);
        for shell in [
            "dash",
            "ksh",
            "csh",
            "/no/such/sh",
            to_text(&wrapper).as_str(),
        ] {
            assert!(interactive_shell_accepts_line(&at_limit, shell), "{shell}");
            assert!(
                !interactive_shell_accepts_line(&over_limit, shell),
                "{shell}"
            );
        }
        // Only shells this platform knows to edit their own line take more:
        // bash, zsh and fish on Linux, none on macOS.
        for (shell, name) in [
            ("bash", "bash"),
            ("zsh", "zsh"),
            ("fish", "fish"),
            (to_text(&bash).as_str(), "bash"),
            (to_text(&sh).as_str(), "bash"),
        ] {
            assert!(interactive_shell_accepts_line(&at_limit, shell), "{shell}");
            assert_eq!(
                interactive_shell_accepts_line(&over_limit, shell),
                LINE_EDITING_SHELLS.contains(&name),
                "{shell}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pane_shell_process_names_reject_exec_replacement_programs() {
        for shell in ["bash", "-zsh", "/bin/fish", "pwsh", "powershell.exe"] {
            assert!(is_pane_shell_process_name(shell), "{shell}");
        }
        for program in ["vim", "nvim", "cargo", "test-runner", "opencode"] {
            assert!(!is_pane_shell_process_name(program), "{program}");
        }
    }

    #[test]
    fn detached_custom_command_preserves_unix_login_shell_flag() {
        let cmd = detached_custom_command_process("echo hello");
        assert_eq!(cmd.get_program(), std::ffi::OsStr::new("/bin/sh"));
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            [
                std::ffi::OsStr::new("-lc"),
                std::ffi::OsStr::new("echo hello")
            ]
        );
    }

    #[test]
    fn pane_custom_command_builder_preserves_unix_shell_flag() {
        let expected: Vec<std::ffi::OsString> =
            vec!["/bin/sh".into(), "-c".into(), "echo hello".into()];
        assert_eq!(
            pane_custom_command_pty_builder("echo hello").get_argv(),
            &expected
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn parse_agent_env_hint_accepts_known_agents() {
        assert_eq!(
            parse_agent_env_hint(b"PATH=/bin\0HERDR_AGENT=claude\0TERM=xterm\0"),
            Some(crate::detect::Agent::Claude)
        );
        assert_eq!(
            parse_agent_env_hint(b"HERDR_AGENT=codex"),
            Some(crate::detect::Agent::Codex)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn parse_agent_env_hint_ignores_missing_or_unknown_agents() {
        assert_eq!(parse_agent_env_hint(b"PATH=/bin\0TERM=xterm\0"), None);
        assert_eq!(parse_agent_env_hint(b"HERDR_AGENT=not-an-agent\0"), None);
    }

    fn placeholder_restore_argv() -> [String; 5] {
        [
            "claude",
            "--model",
            "placeholder",
            "--resume",
            "placeholder-id",
        ]
        .map(String::from)
    }

    #[cfg(unix)]
    #[test]
    fn restore_command_starts_with_a_space_that_keeps_it_out_of_shell_history() {
        let argv = placeholder_restore_argv();
        for shell in ["bash", "zsh", "fish", "sh"] {
            assert_eq!(
                interactive_shell_restore_command(&argv, shell),
                interactive_shell_command(&argv, shell).map(|command| format!(" {command}")),
                "{shell}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn restore_command_is_the_shell_command_unchanged_on_windows() {
        let argv = placeholder_restore_argv();
        for shell in ["pwsh", "powershell", "cmd", "bash"] {
            assert_eq!(
                interactive_shell_restore_command(&argv, shell),
                interactive_shell_command(&argv, shell),
                "{shell}"
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn interactive_shell_command_quotes_for_posix_and_powershell() {
        let argv = vec![
            "pi".into(),
            String::new(),
            "two words".into(),
            "a'b".into(),
            "$HOME".into(),
            "semi;colon".into(),
            "@options".into(),
        ];
        assert_eq!(
            interactive_shell_command(&argv, "bash").as_deref(),
            Some("pi '' 'two words' 'a'\\''b' '$HOME' 'semi;colon' @options")
        );
        assert_eq!(
            interactive_shell_command(&argv, "pwsh").as_deref(),
            Some("pi '' 'two words' 'a''b' '$HOME' 'semi;colon' '@options'")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn interactive_shell_command_quotes_backslashes_for_fish() {
        let argv = vec![
            "claude".into(),
            "--settings".into(),
            r#"{"pattern":"\\d"}"#.into(),
            r"trailing\".into(),
            "a'b".into(),
        ];
        assert_eq!(
            interactive_shell_command(&argv, "fish").as_deref(),
            Some(r#"claude --settings '{"pattern":"\\\\d"}' 'trailing\\' 'a\'b'"#)
        );
        assert_eq!(
            interactive_shell_command(&argv, "/usr/bin/fish"),
            interactive_shell_command(&argv, "fish")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn interactive_shell_command_escapes_history_for_csh_and_tcsh() {
        let argv = vec![
            "claude".into(),
            "--append-system-prompt".into(),
            "wow! a'b".into(),
            "plain".into(),
        ];
        for shell in ["csh", "tcsh", "-tcsh"] {
            assert_eq!(
                interactive_shell_command(&argv, shell).as_deref(),
                Some(r"claude --append-system-prompt 'wow'\!' a'\''b' plain"),
                "{shell}"
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn interactive_shell_command_quotes_apostrophes_for_nu_elvish_and_xonsh() {
        let argv = vec![
            "claude".into(),
            "--append-system-prompt".into(),
            r#"don't "dq" \d $HOME"#.into(),
            "plain".into(),
        ];
        for (shell, expected) in [
            (
                "nu",
                r#"claude --append-system-prompt "don't \"dq\" \\d $HOME" plain"#,
            ),
            (
                "/usr/bin/nu",
                r#"claude --append-system-prompt "don't \"dq\" \\d $HOME" plain"#,
            ),
            (
                "elvish",
                r#"claude --append-system-prompt 'don''t "dq" \d $HOME' plain"#,
            ),
            (
                "xonsh",
                r#"claude --append-system-prompt 'don\'t "dq" \\d $HOME' plain"#,
            ),
        ] {
            assert_eq!(
                interactive_shell_command(&argv, shell).as_deref(),
                Some(expected),
                "{shell}"
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn interactive_shell_command_keeps_posix_quoting_for_sh_family_and_unknown_shells() {
        let argv = vec![
            "claude".into(),
            r#"{"pattern":"\\d"}"#.into(),
            "wow!".into(),
            "a'b".into(),
        ];
        for shell in [
            "bash",
            "zsh",
            "sh",
            "/bin/sh",
            "dash",
            "ksh",
            "mksh",
            "my-shell-wrapper",
        ] {
            assert_eq!(
                interactive_shell_command(&argv, shell).as_deref(),
                Some(r#"claude '{"pattern":"\\d"}' 'wow!' 'a'\''b'"#),
                "{shell}"
            );
        }
    }

    #[test]
    fn read_limited_reader_returns_complete_data_under_limit() {
        let input = std::io::Cursor::new(b"image".to_vec());
        assert_eq!(
            read_limited_reader(input, 16).expect("limited read"),
            LimitedRead::Complete(b"image".to_vec())
        );
    }

    #[test]
    fn read_limited_reader_returns_empty_for_empty_input() {
        let input = std::io::Cursor::new(Vec::<u8>::new());
        assert_eq!(
            read_limited_reader(input, 16).expect("limited read"),
            LimitedRead::Empty
        );
    }

    #[test]
    fn read_limited_reader_accepts_data_exactly_at_limit() {
        let input = std::io::Cursor::new(b"four".to_vec());
        assert_eq!(
            read_limited_reader(input, 4).expect("limited read"),
            LimitedRead::Complete(b"four".to_vec())
        );
    }

    #[test]
    fn read_limited_reader_rejects_data_over_limit() {
        let input = std::io::Cursor::new(b"oversized".to_vec());
        assert_eq!(
            read_limited_reader(input, 4).expect("limited read"),
            LimitedRead::Oversized
        );
    }

    #[test]
    fn read_limited_reader_retries_interrupted_reads() {
        struct InterruptedOnce {
            interrupted: bool,
            inner: std::io::Cursor<Vec<u8>>,
        }

        impl std::io::Read for InterruptedOnce {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.inner.read(buffer)
            }
        }

        let input = InterruptedOnce {
            interrupted: false,
            inner: std::io::Cursor::new(b"image".to_vec()),
        };
        assert_eq!(
            read_limited_reader(input, 16).expect("limited read"),
            LimitedRead::Complete(b"image".to_vec())
        );
    }
}

#[cfg(not(unix))]
pub(crate) fn shared_ssh_control_path(
    _namespace: &std::path::Path,
    _target: &str,
) -> std::io::Result<std::path::PathBuf> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "interactive SSH recovery requires Unix OpenSSH multiplexing",
    ))
}
