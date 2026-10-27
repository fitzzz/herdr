//! Sound notifications for agent state changes.
//!
//! Embeds mp3 files in the binary and plays them via system audio tools.
//! Uses afplay (macOS), Windows MediaPlayer, or decoder-capable Linux audio
//! players — no Rust audio dependencies.

use std::fs::{File, OpenOptions, TryLockError};
#[cfg(not(any(windows, target_os = "macos")))]
use std::io::{Read, Result as IoResult};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[cfg(not(any(windows, target_os = "macos")))]
use std::time::{Duration, Instant};

use tracing::warn;

const DISABLE_SOUND_ENV: &str = "HERDR_DISABLE_SOUND";
#[cfg(any(windows, test))]
const WINDOWS_SOUND_PATH_ENV: &str = "HERDR_SOUND_PATH";
#[cfg(not(any(windows, target_os = "macos")))]
const AUDIO_PLAYER_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(not(any(windows, target_os = "macos")))]
const AUDIO_PLAYER_POLL_INTERVAL: Duration = Duration::from_millis(25);
const SOUND_DIR: &str = "sound";
const PLAYER_LOCK_FILE: &str = "player.lock";

static SOUND_DONE: &[u8] = include_bytes!("../assets/sounds/done.mp3");
static SOUND_REQUEST: &[u8] = include_bytes!("../assets/sounds/request.mp3");

/// Which notification sound to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// Agent finished work (transitioned to Idle).
    Done,
    /// Agent needs input (transitioned to Blocked).
    Request,
}

/// Play a notification sound in a background thread.
/// Silently does nothing if no audio player is available.
///
/// Every attached client receives the same agent transitions, so playback is
/// gated by a lock file in Herdr's state directory, shared by every client on
/// this machine whichever server or session it is attached to. Whoever holds
/// it plays; requests that arrive while a sound is
/// playing are dropped, which also coalesces a burst of transitions into one
/// sound. The OS releases the lock when its holder exits, even mid-play.
pub fn play(sound: Sound, config: &crate::config::SoundConfig) {
    if sound_playback_disabled_by_env() {
        return;
    }

    let dir = crate::config::state_dir().join(SOUND_DIR);
    let lock = match try_lock_player(&dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => return,
        Err(err) => {
            warn!(dir = %dir.display(), err = %err, "sound player lock unavailable");
            return;
        }
    };
    let custom_path = config.path_for(sound);
    std::thread::spawn(move || {
        play_locked(&dir, sound, custom_path.as_deref(), play_file);
        drop(lock);
    });
}

fn sound_playback_disabled_by_env() -> bool {
    std::env::var_os(DISABLE_SOUND_ENV).is_some() || std::env::var_os("NEXTEST").is_some()
}

/// Returns the held player lock, or `None` when another player holds it.
fn try_lock_player(dir: &Path) -> std::io::Result<Option<File>> {
    std::fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(PLAYER_LOCK_FILE))?;
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => Err(err),
    }
}

/// Plays `sound` with `play_path`. The caller must hold the player lock in `dir`.
fn play_locked(
    dir: &Path,
    sound: Sound,
    custom_path: Option<&Path>,
    play_path: impl Fn(&Path) -> Result<(), String>,
) {
    if let Some(path) = custom_path {
        match play_path(path) {
            Ok(()) => return,
            Err(err) => {
                warn!(path = %path.display(), sound = ?sound, err = %err, "custom sound playback failed, falling back to built-in sound")
            }
        }
    }

    let result = embedded_sound_path(dir, sound)
        .map_err(|err| err.to_string())
        .and_then(|path| play_path(&path));
    if let Err(err) = result {
        warn!(sound = ?sound, err = %err, "sound playback failed");
    }
}

/// Writes the embedded sound to a fixed file in `dir` when it is missing or
/// differs, because the supported audio players need a file path. The caller
/// must hold the player lock, so the fixed staging name cannot collide.
fn embedded_sound_path(dir: &Path, sound: Sound) -> std::io::Result<PathBuf> {
    let (name, data) = match sound {
        Sound::Done => ("done.mp3", SOUND_DONE),
        Sound::Request => ("request.mp3", SOUND_REQUEST),
    };
    let path = dir.join(name);
    if std::fs::read(&path).is_ok_and(|existing| existing == data) {
        return Ok(path);
    }
    let staging = dir.join(format!("{name}.tmp"));
    std::fs::write(&staging, data)?;
    std::fs::rename(&staging, &path)?;
    Ok(path)
}

fn play_file(path: &Path) -> Result<(), String> {
    match run_player(path) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(playback_error(&output)),
        Err(err) => Err(err),
    }
}

fn playback_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        format!("player exited with {}", output.status)
    } else {
        format!("player exited with {}: {stderr}", output.status)
    }
}

#[cfg(windows)]
fn run_player(path: &Path) -> Result<Output, String> {
    run_windows_player(path)
}

#[cfg(target_os = "macos")]
fn run_player(path: &Path) -> Result<Output, String> {
    Command::new("afplay")
        .arg(path)
        .output()
        .map_err(|e| format!("no audio player available: {e}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn run_player(path: &Path) -> Result<Output, String> {
    run_linux_player(path)
}

#[cfg(any(windows, test))]
fn windows_media_player_script() -> &'static str {
    r#"
$ErrorActionPreference = 'Stop'
$Path = [Environment]::GetEnvironmentVariable('HERDR_SOUND_PATH', 'Process')
if ([string]::IsNullOrWhiteSpace($Path)) { throw 'HERDR_SOUND_PATH is not set' }
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase
$resolved = (Resolve-Path -LiteralPath $Path).ProviderPath
$script:player = [System.Windows.Media.MediaPlayer]::new()
$script:frame = [System.Windows.Threading.DispatcherFrame]::new()
$script:timer = [System.Windows.Threading.DispatcherTimer]::new()
$script:timer.Interval = [TimeSpan]::FromSeconds(15)
$script:failed = $null
$script:timedOut = $false
$script:player.add_MediaOpened({ $script:player.Play() })
$script:player.add_MediaEnded({ $script:frame.Continue = $false })
$script:player.add_MediaFailed({
    param($sender, $eventArgs)
    $script:failed = $eventArgs.ErrorException
    $script:frame.Continue = $false
})
$script:timer.add_Tick({
    $script:timedOut = $true
    $script:frame.Continue = $false
})
try {
    $script:player.Open([Uri]::new($resolved))
    $script:timer.Start()
    [System.Windows.Threading.Dispatcher]::PushFrame($script:frame)
} finally {
    $script:timer.Stop()
    $script:player.Close()
}
if ($script:failed) { throw "sound media failed: $($script:failed.Message)" }
if ($script:timedOut) { throw 'sound playback timed out' }
"#
}

/// Builds the same player script with a shorter timer for tests. Production
/// keeps the fixed 15 second bound in `windows_media_player_script`.
#[cfg(test)]
fn windows_media_player_script_with_timeout(timeout_seconds: u64) -> String {
    windows_media_player_script().replace(
        "FromSeconds(15)",
        &format!("FromSeconds({timeout_seconds})"),
    )
}

#[cfg(any(windows, test))]
fn windows_player_command(path: &Path, script: &str) -> Command {
    let mut command = crate::noninteractive_process::command("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env(WINDOWS_SOUND_PATH_ENV, path);
    command
}

#[cfg(windows)]
fn run_windows_player(path: &Path) -> Result<Output, String> {
    windows_player_command(path, windows_media_player_script())
        .output()
        .map_err(|e| format!("Windows MediaPlayer playback failed: {e}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
#[derive(Debug, Clone, Copy)]
struct AudioPlayer {
    program: &'static str,
    args: &'static [&'static str],
}

#[cfg(not(any(windows, target_os = "macos")))]
impl AudioPlayer {
    fn output(self, path: &Path) -> std::io::Result<Output> {
        self.output_with_timeout(path, AUDIO_PLAYER_TIMEOUT)
    }

    fn output_with_timeout(self, path: &Path, timeout: Duration) -> std::io::Result<Output> {
        let mut child = Command::new(self.program)
            .args(self.args)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let Some(stdout) = child.stdout.take() else {
            terminate_and_reap(&mut child)?;
            return Err(std::io::Error::other("audio player stdout was not piped"));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_and_reap(&mut child)?;
            return Err(std::io::Error::other("audio player stderr was not piped"));
        };
        let stdout_reader = read_output(stdout);
        let stderr_reader = read_output(stderr);
        let deadline = Instant::now() + timeout;

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let (stdout, stderr) = finish_output(stdout_reader, stderr_reader)?;
                    return Ok(Output {
                        status,
                        stdout,
                        stderr,
                    });
                }
                Ok(None) => {}
                Err(wait_err) => {
                    let cleanup_result = terminate_and_reap(&mut child);
                    let _ = finish_output(stdout_reader, stderr_reader);
                    cleanup_result?;
                    return Err(wait_err);
                }
            }

            let now = Instant::now();
            if now >= deadline {
                let cleanup_result = terminate_and_reap(&mut child);
                let _ = finish_output(stdout_reader, stderr_reader);
                cleanup_result?;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("{} playback timed out after {timeout:?}", self.program),
                ));
            }

            std::thread::sleep((deadline - now).min(AUDIO_PLAYER_POLL_INTERVAL));
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_output<R>(mut reader: R) -> std::thread::JoinHandle<IoResult<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output)?;
        Ok(output)
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn finish_output(
    stdout_reader: std::thread::JoinHandle<IoResult<Vec<u8>>>,
    stderr_reader: std::thread::JoinHandle<IoResult<Vec<u8>>>,
) -> IoResult<(Vec<u8>, Vec<u8>)> {
    let stdout = stdout_reader
        .join()
        .map_err(|_| std::io::Error::other("audio player stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| std::io::Error::other("audio player stderr reader panicked"))??;
    Ok((stdout, stderr))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn terminate_and_reap(child: &mut std::process::Child) -> std::io::Result<()> {
    if let Err(kill_err) = child.kill() {
        if child.try_wait()?.is_none() {
            return Err(kill_err);
        }
    }
    child.wait().map(|_| ())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn linux_audio_players() -> &'static [AudioPlayer] {
    // Do not add bare aplay here. It does not decode MP3 and plays MP3 bytes as raw PCM.
    &[
        AudioPlayer {
            program: "paplay",
            args: &[],
        },
        AudioPlayer {
            program: "pw-play",
            args: &[],
        },
        AudioPlayer {
            program: "ffplay",
            args: &["-nodisp", "-autoexit", "-loglevel", "quiet"],
        },
        AudioPlayer {
            program: "mpg123",
            args: &["-q"],
        },
        AudioPlayer {
            program: "mpv",
            args: &["--no-video", "--really-quiet"],
        },
    ]
}

#[cfg(not(any(windows, target_os = "macos")))]
fn run_linux_player(path: &Path) -> Result<Output, String> {
    let mut errors = Vec::new();

    for player in linux_audio_players() {
        match player.output(path) {
            Ok(output) if output.status.success() => return Ok(output),
            Ok(output) => errors.push(player_error(*player, &output)),
            Err(err) => errors.push(format!("{} failed: {err}", player.program)),
        }
    }

    Err(format!(
        "no mp3-capable audio player available: {}",
        errors.join("; ")
    ))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn player_error(player: AudioPlayer, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();

    if stderr.is_empty() {
        format!("{} exited with {}", player.program, output.status)
    } else {
        format!("{} exited with {}: {stderr}", player.program, output.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    fn unique_test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "herdr-sound-test-{name}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn dir_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn player_lock_admits_one_holder_until_released() {
        let dir = unique_test_dir("lock");

        let held = try_lock_player(&dir)
            .unwrap()
            .expect("first client gets the lock");
        assert!(
            try_lock_player(&dir).unwrap().is_none(),
            "a second client must not play while the first one is playing"
        );
        drop(held);
        assert!(try_lock_player(&dir).unwrap().is_some());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn simultaneous_requests_from_many_clients_start_one_player() {
        const CLIENTS: usize = 12;
        let dir = unique_test_dir("burst");
        let barrier = Arc::new(Barrier::new(CLIENTS));
        let attempted = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));

        let clients: Vec<_> = (0..CLIENTS)
            .map(|_| {
                let (dir, barrier) = (dir.clone(), barrier.clone());
                let (attempted, started) = (attempted.clone(), started.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let lock = try_lock_player(&dir).unwrap();
                    attempted.fetch_add(1, Ordering::SeqCst);
                    let Some(lock) = lock else {
                        return;
                    };
                    play_locked(&dir, Sound::Done, None, |_| {
                        started.fetch_add(1, Ordering::SeqCst);
                        // Keep playing until every other client has asked to play.
                        while attempted.load(Ordering::SeqCst) < CLIENTS {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        Ok(())
                    });
                    drop(lock);
                })
            })
            .collect();
        for client in clients {
            client.join().unwrap();
        }

        assert_eq!(started.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn embedded_sounds_play_from_stable_files_without_leftovers() {
        let dir = unique_test_dir("stable");
        let played = Mutex::new(Vec::new());
        let record = |path: &Path| {
            played.lock().unwrap().push(path.to_path_buf());
            Ok(())
        };

        for _ in 0..3 {
            play_locked(&dir, Sound::Done, None, record);
            play_locked(&dir, Sound::Request, None, record);
        }

        let played = played.into_inner().unwrap();
        assert_eq!(played.len(), 6);
        assert!(played
            .iter()
            .step_by(2)
            .all(|path| path == &dir.join("done.mp3")));
        assert!(played
            .iter()
            .skip(1)
            .step_by(2)
            .all(|path| path == &dir.join("request.mp3")));
        assert_eq!(dir_entries(&dir), ["done.mp3", "request.mp3"]);
        assert_eq!(std::fs::read(dir.join("done.mp3")).unwrap(), SOUND_DONE);
        assert_eq!(
            std::fs::read(dir.join("request.mp3")).unwrap(),
            SOUND_REQUEST
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn embedded_sound_file_is_replaced_when_its_content_differs() {
        let dir = unique_test_dir("replace");
        std::fs::write(dir.join("done.mp3"), b"from another herdr version").unwrap();

        play_locked(&dir, Sound::Done, None, |_| Ok(()));

        assert_eq!(std::fs::read(dir.join("done.mp3")).unwrap(), SOUND_DONE);
        assert_eq!(dir_entries(&dir), ["done.mp3"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_custom_sound_falls_back_to_embedded_file() {
        let dir = unique_test_dir("custom");
        let custom = dir.join("custom.wav");
        let played = Mutex::new(Vec::new());

        play_locked(&dir, Sound::Request, Some(&custom), |path| {
            played.lock().unwrap().push(path.to_path_buf());
            if path == custom {
                Err("unsupported".into())
            } else {
                Ok(())
            }
        });

        assert_eq!(
            played.into_inner().unwrap(),
            [custom.clone(), dir.join("request.mp3")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn linux_audio_players_are_mp3_capable() {
        let programs: Vec<&str> = linux_audio_players()
            .iter()
            .map(|player| player.program)
            .collect();

        assert_eq!(programs, ["paplay", "pw-play", "ffplay", "mpg123", "mpv"]);
        assert!(!programs.contains(&"aplay"));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn linux_audio_player_does_not_wait_forever() {
        let dir = unique_test_dir("timeout");
        let pid_path = dir.join("player.pid");
        let player = AudioPlayer {
            program: "sh",
            args: &[
                "-c",
                "printf '%s' \"$$\" > \"$1\"; exec sleep 2",
                "herdr-sound-timeout-test",
            ],
        };
        let result = player.output_with_timeout(&pid_path, Duration::from_millis(100));
        let pid = std::fs::read_to_string(&pid_path)
            .expect("hanging test player should record its process ID");
        let _ = std::fs::remove_dir_all(dir);

        let err = result.expect_err("hanging audio player should time out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        let status = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .expect("test should inspect the timed-out player PID");
        assert!(
            !status.success(),
            "timed-out audio player should be terminated and reaped"
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn linux_audio_player_preserves_completed_output() {
        let player = AudioPlayer {
            program: "sh",
            args: &[
                "-c",
                "i=0; while [ \"$i\" -lt 8192 ]; do printf 0123456789abcdef; i=$((i + 1)); done; i=0; while [ \"$i\" -lt 8192 ]; do printf fedcba9876543210; i=$((i + 1)); done >&2; exit 7",
                "herdr-sound-output-test",
            ],
        };

        let output = player
            .output_with_timeout(Path::new("unused.mp3"), Duration::from_secs(5))
            .expect("completed audio player should return its output");

        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout.len(), 131_072);
        assert_eq!(output.stderr.len(), 131_072);
        assert!(output.stdout.starts_with(b"0123456789abcdef"));
        assert!(output.stderr.starts_with(b"fedcba9876543210"));
    }

    #[test]
    fn windows_media_player_uses_process_environment_and_dispatcher() {
        let script = windows_media_player_script();
        let path = Path::new(r"C:\sound dir\döne.mp3");
        let command = windows_player_command(path, script);
        let env_path = command.get_envs().find_map(|(key, value)| {
            (key == std::ffi::OsStr::new(WINDOWS_SOUND_PATH_ENV))
                .then_some(value)
                .flatten()
        });

        assert!(script.contains("GetEnvironmentVariable('HERDR_SOUND_PATH', 'Process')"));
        assert!(!script.contains("param([string]$Path)"));
        assert!(script.contains("Resolve-Path -LiteralPath $Path"));
        assert!(script.contains("Dispatcher]::PushFrame"));
        assert!(script.contains("add_MediaEnded"));
        assert!(script.contains("add_MediaFailed"));
        assert!(script.contains("FromSeconds(15)"));
        assert!(windows_media_player_script_with_timeout(2).contains("FromSeconds(2)"));
        assert_eq!(env_path, Some(path.as_os_str()));
        assert!(!command.get_args().any(|arg| arg == path.as_os_str()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_media_player_reports_invalid_media() {
        let _lock = crate::integration::integration_env_lock();
        let dir = unique_test_dir("invalid-media");
        let path = dir.join("invalid.mp3");
        std::fs::write(&path, b"not an mp3").unwrap();
        let script = windows_media_player_script_with_timeout(2);
        // Whether the host media stack raises MediaFailed before the playback
        // timeout is environment-dependent, so use a short test-only timer and
        // accept either terminal error instead of the production 15 second wait.
        let output = windows_player_command(&path, &script).output().unwrap();
        let _ = std::fs::remove_dir_all(dir);

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("sound media failed") || stderr.contains("sound playback timed out"),
            "stderr should report why playback stopped: {stderr}"
        );
    }
}
