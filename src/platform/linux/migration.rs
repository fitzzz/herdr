//! One-shot live migration authentication. No process control or discovery.
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::platform::{MigrationFileStamp, MigrationProcessStamp};

fn process_start_ticks(stat: &str) -> io::Result<u64> {
    let fields = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("malformed process stat"))?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    if matches!(fields.first().copied(), Some("Z" | "X" | "x")) {
        return Err(io::Error::other("migration process has exited"));
    }
    fields
        .get(19)
        .ok_or_else(|| io::Error::other("process stat lacks start ticks"))?
        .parse()
        .map_err(io::Error::other)
}

pub(crate) fn migration_process_stamp(pid: u32) -> io::Result<MigrationProcessStamp> {
    if pid == 0 {
        return Err(io::Error::other("migration process PID is zero"));
    }
    let base = std::path::PathBuf::from(format!("/proc/{pid}"));
    let first = std::fs::read_to_string(base.join("stat"))?;
    let start_ticks = process_start_ticks(&first)?;
    let stamp = MigrationProcessStamp {
        pid,
        uid: std::fs::metadata(&base)?.uid(),
        start_ticks,
        boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .to_string(),
        pid_namespace: std::fs::read_link(base.join("ns/pid"))?
            .to_string_lossy()
            .into_owned(),
        mount_namespace: std::fs::read_link(base.join("ns/mnt"))?
            .to_string_lossy()
            .into_owned(),
    };
    if process_start_ticks(&std::fs::read_to_string(base.join("stat"))?)? != start_ticks {
        return Err(io::Error::other("migration process identity changed"));
    }
    Ok(stamp)
}

pub(crate) fn migration_parent_pid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

pub(crate) fn migration_executable_digest(pid: u32) -> io::Result<String> {
    let mut file = File::open(format!("/proc/{pid}/exe"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn migration_peer_stamp(stream: &UnixStream) -> io::Result<MigrationProcessStamp> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if size as usize != std::mem::size_of::<libc::ucred>() || credentials.pid <= 0 {
        return Err(io::Error::other("migration peer credentials unavailable"));
    }
    let stamp = migration_process_stamp(credentials.pid as u32)?;
    if stamp.uid != credentials.uid {
        return Err(io::Error::other("migration peer UID differs"));
    }
    Ok(stamp)
}

pub(crate) fn migration_socket_stamp(path: &Path) -> io::Result<MigrationFileStamp> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(io::Error::other("migration path is not a socket"));
    }
    Ok(MigrationFileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
    })
}

pub(crate) fn migration_file_stamp(file: &File) -> io::Result<MigrationFileStamp> {
    let metadata = file.metadata()?;
    Ok(MigrationFileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
    })
}

/// Take ownership only of the three explicit descriptors supplied by the
/// exec-only shim. All are read-only; directory writes use exclusive openat.
pub(crate) fn migration_attempt_files(fds: [RawFd; 3]) -> io::Result<[File; 3]> {
    if fds.iter().any(|fd| *fd < 3) || fds[0] == fds[1] || fds[0] == fds[2] || fds[1] == fds[2] {
        return Err(io::Error::other(
            "migration descriptors must be distinct and nonstandard",
        ));
    }
    // from_raw_fd requires an open descriptor; malformed explicit arguments
    // must not turn a validation error into an invalid OwnedFd drop.
    for fd in fds {
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            return Err(io::Error::other("migration descriptor is not open"));
        }
    }
    let files = fds.map(|fd| unsafe { File::from_raw_fd(fd) });
    for (index, file) in files.iter().enumerate() {
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        let metadata = file.metadata()?;
        let correct_type = if index == 2 {
            metadata.is_dir()
        } else {
            metadata.is_file()
        };
        let mode = if index == 2 { 0o700 } else { 0o400 };
        if flags < 0
            || flags & libc::O_ACCMODE != libc::O_RDONLY
            || !correct_type
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != mode
        {
            return Err(io::Error::other(
                "migration descriptor type, owner or mode differs",
            ));
        }
    }
    Ok(files)
}

pub(crate) fn migration_create_record(
    directory: &File,
    name: &str,
    bytes: &[u8],
) -> io::Result<()> {
    // Names are fixed phase records, never user-supplied paths.
    if name.contains('/') {
        return Err(io::Error::other("migration record name is not local"));
    }
    let name = CString::new(name).map_err(io::Error::other)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o400,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes)?;
    file.sync_all()
}

pub(crate) fn migration_validate_master_fd(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    let mut number: libc::c_uint = 0;
    if flags < 0 || unsafe { libc::ioctl(fd, libc::TIOCGPTN, &mut number) } < 0 {
        return Err(io::Error::other(
            "migration descriptor is not a usable PTY master",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_stat_handles_parentheses_and_rejects_zombies() {
        let tail = std::iter::repeat_n("0", 18).collect::<Vec<_>>().join(" ");
        assert_eq!(
            process_start_ticks(&format!("42 (a ) name) S {tail} 123 0")).unwrap(),
            123
        );
        assert!(process_start_ticks(&format!("42 (a) Z {tail} 123 0")).is_err());
        assert!(process_start_ticks("broken").is_err());
    }
}
