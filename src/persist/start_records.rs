//! The tracked launcher journal. Layout saves and their rollback do not own these records.
//! Calls that touch the filesystem run on the blocking side of the server.

use std::collections::HashSet;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, MutexGuard,
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const RECORD_BYTES: u64 = 4096;
const DIR: &str = "agent-starts";

pub(crate) fn canonical_request_id(id: &str) -> bool {
    id.len() == 36
        && id != "00000000-0000-0000-0000-000000000000"
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct StartShell {
    pub(crate) pid: u32,
    pub(crate) started: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct StartRecord {
    version: u32,
    start_request_id: String,
    pub(crate) terminal_id: String,
    nonce: String,
    shell: StartShell,
    pub(crate) disposition: StartDisposition,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum StartDisposition {
    Pending,
    Authorized { launch_incarnation: String },
    DiscardedAtRestore { restored_at: u64 },
}

impl StartRecord {
    fn readable(&self, id: &str) -> bool {
        self.version == VERSION
            && self.start_request_id == id
            && !self.terminal_id.is_empty()
            && !self.nonce.is_empty()
            && self.shell.pid != 0
            && match &self.disposition {
                StartDisposition::Pending => true,
                StartDisposition::Authorized { launch_incarnation } => {
                    crate::terminal::launch_incarnation::is_canonical_token(launch_incarnation)
                }
                StartDisposition::DiscardedAtRestore { restored_at } => *restored_at > 0,
            }
    }
}

pub(crate) struct StartRecords {
    root: PathBuf,
    // The lock belongs to the file description. A handoff sends a duplicate
    // while the source still owns it, so there is no cold-writer gap at commit.
    writer: File,
    used: AtomicBool,
    unconfirmed: Mutex<HashSet<String>>,
}

impl StartRecords {
    pub(crate) fn open_cold(session: &Path) -> io::Result<Self> {
        let records = Self::open_live(session)?;
        records.discard_pending()?;
        Ok(records)
    }

    pub(crate) fn open_live(session: &Path) -> io::Result<Self> {
        fs::create_dir_all(session)?;
        let root = session.join(DIR);
        match DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
        let writer = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(&root)?;
        writer.try_lock()?;
        let records = Self::with_writer(root, writer)?;
        // Confirm the journal directory entry as well as any visible previous
        // record rename, including a retry after a failed directory sync.
        File::open(session)?.sync_all()?;
        records.writer.sync_all()?;
        records.note_existing_history()?;
        Ok(records)
    }

    pub(crate) fn import_handoff(session: &Path, writer: File) -> io::Result<Self> {
        // The authenticated source hands over the same locked open description.
        // Never reacquire a writer after releasing the source's fence.
        let records = Self::with_writer(session.join(DIR), writer)?;
        records.writer.try_lock()?;
        records.writer.sync_all()?;
        records.used.store(true, Ordering::Relaxed);
        Ok(records)
    }

    pub(crate) fn has_history(&self) -> bool {
        self.used.load(Ordering::Relaxed)
    }

    fn note_existing_history(&self) -> io::Result<()> {
        // Any existing entry, including a malformed record or uncertain staging
        // file, calls for fencing the same journal through live handoff.
        if fs::read_dir(&self.root)?.next().transpose()?.is_some() {
            self.used.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(crate) fn writer(&self) -> &File {
        &self.writer
    }

    fn with_writer(root: PathBuf, writer: File) -> io::Result<Self> {
        let actual = writer.metadata()?;
        let expected = fs::symlink_metadata(&root)?;
        if !actual.is_dir()
            || actual.mode() & 0o7777 != 0o700
            || actual.dev() != expected.dev()
            || actual.ino() != expected.ino()
        {
            return Err(io::Error::other(
                "tracked-start writer does not own the private journal",
            ));
        }
        Ok(Self {
            root,
            writer,
            used: AtomicBool::new(false),
            unconfirmed: Mutex::default(),
        })
    }

    pub(crate) fn reserved_id(&self, id: &str) -> io::Result<bool> {
        let _guard = self.lock()?;
        match fs::symlink_metadata(self.path(id)?) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn reserve(
        &self,
        id: &str,
        terminal_id: &str,
        nonce: &str,
        shell: StartShell,
    ) -> io::Result<()> {
        let mut unconfirmed = self.lock()?;
        let path = self.path(id)?;
        if unconfirmed.contains(id) {
            return Err(io::Error::other("tracked-start write is unconfirmed"));
        }
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "start ID already recorded",
                ))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let record = StartRecord {
            version: VERSION,
            start_request_id: id.to_string(),
            terminal_id: terminal_id.to_string(),
            nonce: nonce.to_string(),
            shell,
            disposition: StartDisposition::Pending,
        };
        self.used.store(true, Ordering::Relaxed);
        self.commit(&record, &mut unconfirmed)
    }

    pub(crate) fn pending_claim_matches(
        &self,
        id: &str,
        terminal: &str,
        nonce: &str,
        shell: StartShell,
    ) -> io::Result<bool> {
        Ok(self.get(id)?.is_some_and(|record| {
            matches!(record.disposition, StartDisposition::Pending)
                && record.terminal_id == terminal
                && record.nonce == nonce
                && record.shell == shell
        }))
    }

    pub(crate) fn authorize(
        &self,
        id: &str,
        terminal: &str,
        nonce: &str,
        shell: StartShell,
        launch_incarnation: &str,
    ) -> io::Result<()> {
        let mut unconfirmed = self.lock()?;
        let mut record = self.read(id, &unconfirmed)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "tracked start was not admitted")
        })?;
        if !matches!(record.disposition, StartDisposition::Pending)
            || record.terminal_id != terminal
            || record.nonce != nonce
            || record.shell != shell
        {
            return Err(io::Error::other(
                "tracked start no longer awaits this claim",
            ));
        }
        record.disposition = StartDisposition::Authorized {
            launch_incarnation: launch_incarnation.to_string(),
        };
        self.commit(&record, &mut unconfirmed)
    }

    pub(crate) fn get(&self, id: &str) -> io::Result<Option<StartRecord>> {
        self.read(id, &*self.lock()?)
    }

    fn lock(&self) -> io::Result<MutexGuard<'_, HashSet<String>>> {
        self.unconfirmed
            .lock()
            .map_err(|_| io::Error::other("tracked-start writer unavailable"))
    }

    fn path(&self, id: &str) -> io::Result<PathBuf> {
        if !canonical_request_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid tracked-start request ID",
            ));
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    fn read(&self, id: &str, unconfirmed: &HashSet<String>) -> io::Result<Option<StartRecord>> {
        if unconfirmed.contains(id) {
            return Err(io::Error::other("tracked-start write is unconfirmed"));
        }
        let path = self.path(id)?;
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.len() > RECORD_BYTES
            || meta.mode() & 0o7777 != 0o600
            || meta.uid() != self.writer.metadata()?.uid()
        {
            return Err(io::Error::other("invalid tracked-start journal file"));
        }
        let mut content = Vec::new();
        file.take(RECORD_BYTES + 1).read_to_end(&mut content)?;
        if content.len() as u64 > RECORD_BYTES {
            return Err(io::Error::other(
                "tracked-start journal record exceeds its limit",
            ));
        }
        let record: StartRecord = serde_json::from_slice(&content).map_err(io::Error::other)?;
        if !record.readable(id) {
            return Err(io::Error::other(
                "inconsistent tracked-start journal record",
            ));
        }
        Ok(Some(record))
    }

    fn commit(&self, record: &StartRecord, unconfirmed: &mut HashSet<String>) -> io::Result<()> {
        let id = &record.start_request_id;
        if !record.readable(id) {
            return Err(io::Error::other("invalid tracked-start record"));
        }
        let path = self.path(id)?;
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        if bytes.len() as u64 > RECORD_BYTES {
            return Err(io::Error::other(
                "tracked-start journal record exceeds its limit",
            ));
        }
        let stage = path.with_extension("pending");
        match fs::remove_file(&stage) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let result = (|| -> io::Result<()> {
            use std::io::Write;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&stage)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&stage, &path)?;
            self.writer.sync_all()
        })();
        if result.is_err() {
            unconfirmed.insert(id.clone());
        }
        result
    }

    fn discard_pending(&self) -> io::Result<()> {
        let mut unconfirmed = self.lock()?;
        let restored_at = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis(),
        )
        .map_err(io::Error::other)?;
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || !canonical_request_id(id)
            {
                continue;
            }
            let result = (|| -> io::Result<()> {
                if let Some(mut record) = self.read(id, &unconfirmed)? {
                    if matches!(record.disposition, StartDisposition::Pending) {
                        record.disposition = StartDisposition::DiscardedAtRestore { restored_at };
                        self.commit(&record, &mut unconfirmed)?;
                    }
                }
                Ok(())
            })();
            if let Err(err) = result {
                tracing::warn!(request_id = id, %err, "could not recover tracked start record");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "aaaaaaaa-1234-1234-1234-123456789abc";
    const SECOND: &str = "bbbbbbbb-1234-1234-1234-123456789abc";
    const TERMINAL: &str = "original-terminal";
    const SHELL: StartShell = StartShell {
        pid: 12,
        started: 13,
    };

    #[test]
    fn cold_owner_discards_only_the_unclaimed_start() {
        let session = crate::agent_launcher::TestDirectory::new();
        let records = StartRecords::open_cold(session.path()).unwrap();
        let (launch, nonce) = crate::terminal::launch_incarnation::LaunchIncarnation::admit(
            crate::detect::Agent::Codex,
        );
        records.reserve(FIRST, TERMINAL, &nonce, SHELL).unwrap();
        records.reserve(SECOND, TERMINAL, "pending", SHELL).unwrap();
        assert!(StartRecords::open_cold(session.path()).is_err());
        assert_eq!(
            records
                .reserve(SECOND, "different-terminal", "new", SHELL)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists,
        );
        assert!(records
            .authorize(FIRST, TERMINAL, "wrong", SHELL, launch.token())
            .is_err());
        records
            .authorize(FIRST, TERMINAL, &nonce, SHELL, launch.token())
            .unwrap();
        assert!(records
            .authorize(FIRST, TERMINAL, &nonce, SHELL, launch.token())
            .is_err());
        drop(records);

        let records = StartRecords::open_cold(session.path()).unwrap();
        let authorized = records.get(FIRST).unwrap().unwrap();
        assert_eq!(authorized.terminal_id, TERMINAL);
        assert!(matches!(
            authorized.disposition,
            StartDisposition::Authorized { launch_incarnation } if launch_incarnation == launch.token(),
        ));
        let discarded = records.get(SECOND).unwrap().unwrap();
        assert_eq!(discarded.terminal_id, TERMINAL);
        let StartDisposition::DiscardedAtRestore { restored_at } = discarded.disposition else {
            panic!("cold recovery must retire the pending start")
        };
        assert!(records
            .authorize(SECOND, TERMINAL, "pending", SHELL, launch.token())
            .is_err());
        drop(records);

        let records = StartRecords::open_cold(session.path()).unwrap();
        assert!(matches!(
            records.get(SECOND).unwrap().unwrap().disposition,
            StartDisposition::DiscardedAtRestore { restored_at: saved } if saved == restored_at,
        ));
    }

    #[test]
    fn handoff_keeps_the_same_writer_and_leaves_pending_unchanged() {
        let session = crate::agent_launcher::TestDirectory::new();
        let source = StartRecords::open_cold(session.path()).unwrap();
        source.reserve(FIRST, TERMINAL, "pending", SHELL).unwrap();
        let imported =
            StartRecords::import_handoff(session.path(), source.writer().try_clone().unwrap())
                .unwrap();
        assert!(matches!(
            imported.get(FIRST).unwrap().unwrap().disposition,
            StartDisposition::Pending
        ));
        drop(imported);
        assert!(StartRecords::open_cold(session.path()).is_err());
        assert!(matches!(
            source.get(FIRST).unwrap().unwrap().disposition,
            StartDisposition::Pending
        ));
    }

    #[test]
    fn corrupt_record_is_unreadable_but_the_id_stays_reserved() {
        let session = crate::agent_launcher::TestDirectory::new();
        let records = StartRecords::open_cold(session.path()).unwrap();
        records.reserve(FIRST, TERMINAL, "pending", SHELL).unwrap();
        fs::write(records.path(FIRST).unwrap(), b"{").unwrap();
        drop(records);

        let records = StartRecords::open_cold(session.path()).unwrap();
        assert!(records.get(FIRST).is_err());
        assert_eq!(
            records
                .reserve(FIRST, TERMINAL, "new", SHELL)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(records.get(SECOND).unwrap().is_none());
    }
}
