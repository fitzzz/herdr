//! Acknowledged, de-duplicated input streams for one terminal incarnation.
//!
//! Backs the `terminal.input.*` API. Each live terminal runtime owns one
//! table, so stream state ends with the runtime and a recreated terminal gets
//! a new incarnation with no streams. The table is pure data: callers hold its
//! lock across admission, de-duplication and queueing the bytes for the PTY,
//! and report the PTY write outcome back through [`InputStreamTable::complete_write`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

pub(crate) const MAX_STREAMS_PER_TERMINAL: usize = 32;
pub(crate) const RETAINED_SEQUENCES_PER_STREAM: usize = 128;
/// A live handoff carries only the newest digests of each stream: after the
/// writer drained, every admitted write is written, so reconcile needs none of
/// them and only blind resends of recent requests can use one.
pub(crate) const HANDOFF_RETAINED_SEQUENCES_PER_STREAM: usize = 32;
pub(crate) const STREAM_IDLE_EXPIRY: Duration = Duration::from_secs(24 * 60 * 60);

/// Called once with the PTY write outcome of one admitted sequence.
pub(crate) type WriteWaiter = Box<dyn FnOnce(Result<(), String>) + Send>;

/// The first 16 bytes of an [`InputDigest`]: enough to tell a resend from a
/// different payload, and half the size to retain and hand off.
pub(crate) type DigestKey = [u8; 16];

const DIGEST_KEY_LEN: usize = std::mem::size_of::<DigestKey>();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputDigest {
    /// Full SHA-256 in lowercase hex, as reported to the client.
    pub hex: String,
    pub key: DigestKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InputStreamError {
    StaleTarget { current: String },
    StreamNotFound,
    SequenceGap { last_admitted_seq: u64 },
    PayloadMismatch,
    SequenceNotRetained,
    TooManyStreams,
    WriteFailed(String),
}

impl InputStreamError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::StaleTarget { .. } => "stale_target",
            Self::StreamNotFound => "stream_not_found",
            Self::SequenceGap { .. } => "sequence_gap",
            Self::PayloadMismatch => "payload_mismatch",
            Self::SequenceNotRetained => "sequence_not_retained",
            Self::TooManyStreams => "too_many_streams",
            Self::WriteFailed(_) => "write_failed",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::StaleTarget { current } => {
                format!("terminal incarnation changed; current incarnation is {current}")
            }
            Self::StreamNotFound => "input stream not found for this terminal incarnation".into(),
            Self::SequenceGap { last_admitted_seq } => format!(
                "sequence must be {}; last admitted sequence is {last_admitted_seq}",
                last_admitted_seq + 1
            ),
            Self::PayloadMismatch => {
                "sequence was already admitted with a different payload; nothing was written".into()
            }
            Self::SequenceNotRetained => {
                "sequence was already admitted but its payload digest is no longer retained".into()
            }
            Self::TooManyStreams => {
                "terminal has the maximum number of input streams with writes in progress".into()
            }
            Self::WriteFailed(message) => {
                format!("write did not complete and some bytes may have been written: {message}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamPosition {
    pub stream_id: String,
    pub last_admitted_seq: u64,
    pub last_written_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteClass {
    /// The next sequence: queue its bytes, then call `record_admitted`.
    Next,
    /// Same payload, already written to the PTY.
    DuplicateWritten,
    /// Same payload, still being written: call `wait_for`.
    DuplicatePending,
}

enum SequenceOutcome {
    Pending(Vec<WriteWaiter>),
    Written,
    Failed(String),
}

struct RetainedSequence {
    seq: u64,
    digest: DigestKey,
    outcome: SequenceOutcome,
}

struct StreamState {
    last_admitted_seq: u64,
    last_written_seq: u64,
    last_used: Instant,
    retained: VecDeque<RetainedSequence>,
}

impl StreamState {
    fn new(now: Instant) -> Self {
        Self {
            last_admitted_seq: 0,
            last_written_seq: 0,
            last_used: now,
            retained: VecDeque::new(),
        }
    }

    fn has_pending(&self) -> bool {
        self.retained
            .iter()
            .any(|entry| matches!(entry.outcome, SequenceOutcome::Pending(_)))
    }

    fn retained_mut(&mut self, seq: u64) -> Option<&mut RetainedSequence> {
        let first = self.retained.front()?.seq;
        let index = usize::try_from(seq.checked_sub(first)?).ok()?;
        self.retained.get_mut(index)
    }

    /// Drops the oldest digests past the window. A pending write keeps its
    /// digest, and every later one, until it completes.
    fn trim(&mut self) {
        while self.retained.len() > RETAINED_SEQUENCES_PER_STREAM
            && self
                .retained
                .front()
                .is_some_and(|entry| !matches!(entry.outcome, SequenceOutcome::Pending(_)))
        {
            self.retained.pop_front();
        }
    }

    fn position(&self, stream_id: &str) -> StreamPosition {
        StreamPosition {
            stream_id: stream_id.to_owned(),
            last_admitted_seq: self.last_admitted_seq,
            last_written_seq: self.last_written_seq,
        }
    }
}

pub(crate) struct InputStreamTable {
    incarnation: String,
    streams: HashMap<String, StreamState>,
}

impl InputStreamTable {
    pub(crate) fn new() -> Self {
        Self {
            incarnation: mint_token("inc_"),
            streams: HashMap::new(),
        }
    }

    pub(crate) fn incarnation(&self) -> &str {
        &self.incarnation
    }

    pub(crate) fn check_incarnation(
        &self,
        incarnation: Option<&str>,
    ) -> Result<(), InputStreamError> {
        match incarnation {
            Some(incarnation) if incarnation != self.incarnation => {
                Err(InputStreamError::StaleTarget {
                    current: self.incarnation.clone(),
                })
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn open(&mut self, now: Instant) -> Result<StreamPosition, InputStreamError> {
        self.expire_idle(now);
        if self.streams.len() >= MAX_STREAMS_PER_TERMINAL {
            let evict = self
                .streams
                .iter()
                .filter(|(_, stream)| !stream.has_pending())
                .min_by_key(|(_, stream)| stream.last_used)
                .map(|(stream_id, _)| stream_id.clone())
                .ok_or(InputStreamError::TooManyStreams)?;
            self.streams.remove(&evict);
        }
        let stream_id = loop {
            let candidate = mint_token("tis_");
            if !self.streams.contains_key(&candidate) {
                break candidate;
            }
        };
        let stream = StreamState::new(now);
        let position = stream.position(&stream_id);
        self.streams.insert(stream_id, stream);
        Ok(position)
    }

    pub(crate) fn position(
        &mut self,
        stream_id: &str,
        now: Instant,
    ) -> Result<StreamPosition, InputStreamError> {
        self.expire_idle(now);
        let stream = self
            .streams
            .get_mut(stream_id)
            .ok_or(InputStreamError::StreamNotFound)?;
        stream.last_used = now;
        Ok(stream.position(stream_id))
    }

    pub(crate) fn classify_write(
        &mut self,
        stream_id: &str,
        seq: u64,
        digest: &DigestKey,
        now: Instant,
    ) -> Result<WriteClass, InputStreamError> {
        self.expire_idle(now);
        let stream = self
            .streams
            .get_mut(stream_id)
            .ok_or(InputStreamError::StreamNotFound)?;
        stream.last_used = now;
        if seq == stream.last_admitted_seq + 1 {
            return Ok(WriteClass::Next);
        }
        if seq == 0 || seq > stream.last_admitted_seq {
            return Err(InputStreamError::SequenceGap {
                last_admitted_seq: stream.last_admitted_seq,
            });
        }
        let entry = stream
            .retained_mut(seq)
            .ok_or(InputStreamError::SequenceNotRetained)?;
        if &entry.digest != digest {
            return Err(InputStreamError::PayloadMismatch);
        }
        match &entry.outcome {
            SequenceOutcome::Written => Ok(WriteClass::DuplicateWritten),
            SequenceOutcome::Pending(_) => Ok(WriteClass::DuplicatePending),
            SequenceOutcome::Failed(message) => Err(InputStreamError::WriteFailed(message.clone())),
        }
    }

    /// Records the next sequence after its bytes were queued for the PTY.
    pub(crate) fn record_admitted(
        &mut self,
        stream_id: &str,
        seq: u64,
        digest: DigestKey,
        waiter: WriteWaiter,
    ) {
        let Some(stream) = self.streams.get_mut(stream_id) else {
            waiter(Err("input stream disappeared before admission".into()));
            return;
        };
        debug_assert_eq!(seq, stream.last_admitted_seq + 1);
        stream.last_admitted_seq = seq;
        stream.retained.push_back(RetainedSequence {
            seq,
            digest,
            outcome: SequenceOutcome::Pending(vec![waiter]),
        });
        stream.trim();
    }

    /// Adds a waiter to a sequence `classify_write` reported as pending.
    pub(crate) fn wait_for(&mut self, stream_id: &str, seq: u64, waiter: WriteWaiter) {
        let entry = self
            .streams
            .get_mut(stream_id)
            .and_then(|stream| stream.retained_mut(seq));
        match entry.map(|entry| &mut entry.outcome) {
            Some(SequenceOutcome::Pending(waiters)) => waiters.push(waiter),
            Some(SequenceOutcome::Written) => waiter(Ok(())),
            Some(SequenceOutcome::Failed(message)) => waiter(Err(message.clone())),
            None => waiter(Err("input stream disappeared while waiting".into())),
        }
    }

    /// Applies a PTY write outcome and returns the waiters to notify after the
    /// lock is released. Writes on one terminal complete in admission order, so
    /// a written sequence also proves every earlier sequence of its stream.
    #[must_use]
    pub(crate) fn complete_write(
        &mut self,
        stream_id: &str,
        seq: u64,
        result: Result<(), String>,
    ) -> Vec<(WriteWaiter, Result<(), String>)> {
        let Some(stream) = self.streams.get_mut(stream_id) else {
            return Vec::new();
        };
        let mut notify = Vec::new();
        match result {
            Ok(()) => {
                stream.last_written_seq = stream.last_written_seq.max(seq);
                for entry in stream.retained.iter_mut().filter(|entry| entry.seq <= seq) {
                    if matches!(entry.outcome, SequenceOutcome::Pending(_)) {
                        let SequenceOutcome::Pending(waiters) =
                            std::mem::replace(&mut entry.outcome, SequenceOutcome::Written)
                        else {
                            unreachable!("matched pending outcome")
                        };
                        notify.extend(waiters.into_iter().map(|waiter| (waiter, Ok(()))));
                    }
                }
            }
            Err(message) => {
                if let Some(entry) = stream.retained_mut(seq) {
                    if matches!(entry.outcome, SequenceOutcome::Pending(_)) {
                        let SequenceOutcome::Pending(waiters) = std::mem::replace(
                            &mut entry.outcome,
                            SequenceOutcome::Failed(message.clone()),
                        ) else {
                            unreachable!("matched pending outcome")
                        };
                        notify.extend(
                            waiters
                                .into_iter()
                                .map(|waiter| (waiter, Err(message.clone()))),
                        );
                    }
                }
            }
        }
        stream.trim();
        notify
    }

    fn expire_idle(&mut self, now: Instant) {
        self.streams.retain(|_, stream| {
            stream.has_pending()
                || now.saturating_duration_since(stream.last_used) < STREAM_IDLE_EXPIRY
        });
    }

    /// Captures state for a live handoff. Call only after the terminal's PTY
    /// writer quiesced: quiesce drains every queued write, so sequences still
    /// marked pending here were written and are exported as written. Only the
    /// newest [`HANDOFF_RETAINED_SEQUENCES_PER_STREAM`] digests are carried.
    #[cfg(unix)]
    pub(crate) fn export_for_handoff(&self) -> HandoffInputStreams {
        use base64::Engine;

        HandoffInputStreams {
            incarnation: self.incarnation.clone(),
            streams: self
                .streams
                .iter()
                .map(|(stream_id, stream)| {
                    let last_written_seq = stream
                        .retained
                        .iter()
                        .filter(|entry| !matches!(entry.outcome, SequenceOutcome::Failed(_)))
                        .map(|entry| entry.seq)
                        .fold(stream.last_written_seq, u64::max);
                    let skip = stream
                        .retained
                        .len()
                        .saturating_sub(HANDOFF_RETAINED_SEQUENCES_PER_STREAM);
                    let tail = stream.retained.iter().skip(skip);
                    let digests: Vec<u8> = tail.clone().flat_map(|entry| entry.digest).collect();
                    HandoffInputStream {
                        stream_id: stream_id.clone(),
                        last_admitted_seq: stream.last_admitted_seq,
                        last_written_seq,
                        digests: base64::engine::general_purpose::STANDARD_NO_PAD.encode(digests),
                        failed: tail
                            .filter(|entry| matches!(entry.outcome, SequenceOutcome::Failed(_)))
                            .map(|entry| entry.seq)
                            .collect(),
                    }
                })
                .collect(),
        }
    }

    /// Restores exported state. A malformed stream is dropped on its own, so
    /// its client gets `stream_not_found` and can never resume against
    /// sequence numbers the server cannot trust; the terminal keeps its
    /// incarnation and its other streams. Only state with no usable
    /// incarnation yields a fresh one.
    #[cfg(unix)]
    pub(crate) fn from_handoff(state: HandoffInputStreams, now: Instant) -> Self {
        if state.incarnation.is_empty() {
            tracing::warn!("discarding terminal input stream handoff state without incarnation");
            return Self::new();
        }
        let mut seen = HashSet::new();
        let duplicated: HashSet<String> = state
            .streams
            .iter()
            .filter(|stream| !seen.insert(stream.stream_id.as_str()))
            .map(|stream| stream.stream_id.clone())
            .collect();
        let mut streams = HashMap::new();
        for stream in state.streams {
            if streams.len() == MAX_STREAMS_PER_TERMINAL {
                tracing::warn!("dropping terminal input streams past the per-terminal limit");
                break;
            }
            if duplicated.contains(&stream.stream_id) {
                tracing::warn!("dropping duplicated terminal input stream from handoff state");
                continue;
            }
            match imported_stream(&stream, now) {
                Some(imported) => {
                    streams.insert(stream.stream_id, imported);
                }
                None => tracing::warn!("dropping malformed terminal input stream handoff state"),
            }
        }
        Self {
            incarnation: state.incarnation,
            streams,
        }
    }
}

#[cfg(unix)]
fn imported_stream(stream: &HandoffInputStream, now: Instant) -> Option<StreamState> {
    use base64::Engine;

    let digests = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(&stream.digests)
        .ok()?;
    if stream.stream_id.is_empty()
        || stream.last_written_seq > stream.last_admitted_seq
        || digests.len() % DIGEST_KEY_LEN != 0
    {
        return None;
    }
    let count = digests.len() / DIGEST_KEY_LEN;
    if count > RETAINED_SEQUENCES_PER_STREAM || count as u64 > stream.last_admitted_seq {
        return None;
    }
    let first_seq = stream.last_admitted_seq - count as u64 + 1;
    let failed: HashSet<u64> = stream.failed.iter().copied().collect();
    if failed
        .iter()
        .any(|seq| *seq < first_seq || *seq > stream.last_admitted_seq)
    {
        return None;
    }
    let retained = digests
        .chunks_exact(DIGEST_KEY_LEN)
        .zip(first_seq..)
        .map(|(digest, seq)| RetainedSequence {
            seq,
            digest: digest.try_into().unwrap_or_default(),
            outcome: if failed.contains(&seq) {
                SequenceOutcome::Failed("write failed before live handoff".into())
            } else {
                SequenceOutcome::Written
            },
        })
        .collect();
    Some(StreamState {
        last_admitted_seq: stream.last_admitted_seq,
        last_written_seq: stream.last_written_seq,
        last_used: now,
        retained,
    })
}

/// One queued PTY submission whose outcome the completion worker reports.
pub(crate) struct PendingCompletion {
    pub stream_id: String,
    pub seq: u64,
    pub completion: mpsc::Receiver<std::io::Result<()>>,
}

/// Shared handle: the API thread admits writes and one completion worker per
/// terminal reports their outcomes through the same lock. Writes on a terminal
/// complete in admission order, so the worker waits for them one at a time.
#[derive(Clone)]
pub(crate) struct TerminalInputStreams {
    table: Arc<Mutex<InputStreamTable>>,
    /// The worker holds only `table`, so it stops once every handle is gone
    /// and it has reported the writes already handed to it.
    completions: Arc<Mutex<Option<mpsc::Sender<PendingCompletion>>>>,
}

impl TerminalInputStreams {
    pub(crate) fn new() -> Self {
        Self::from_table(InputStreamTable::new())
    }

    pub(crate) fn from_table(table: InputStreamTable) -> Self {
        Self {
            table: Arc::new(Mutex::new(table)),
            completions: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, InputStreamTable> {
        lock_table(&self.table)
    }

    /// The channel to this terminal's completion worker, started on first use.
    /// Call before queueing bytes: when the worker cannot start, nothing has
    /// been written and the request can be refused.
    pub(crate) fn completion_sender(&self) -> std::io::Result<mpsc::Sender<PendingCompletion>> {
        let mut completions = self
            .completions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sender) = completions.as_ref() {
            return Ok(sender.clone());
        }
        let (sender, receiver) = mpsc::channel();
        let table = Arc::clone(&self.table);
        std::thread::Builder::new()
            .name("herdr-input-ack".into())
            .spawn(move || run_completions(&table, receiver))?;
        *completions = Some(sender.clone());
        Ok(sender)
    }

    /// Hands an admitted write to the completion worker. If the worker has
    /// stopped, the write is reported as failed: its bytes were queued, so its
    /// outcome is unknown.
    pub(crate) fn track(
        &self,
        sender: &mpsc::Sender<PendingCompletion>,
        pending: PendingCompletion,
    ) {
        if let Err(mpsc::SendError(pending)) = sender.send(pending) {
            tracing::warn!("terminal input completion worker stopped");
            self.completions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            self.complete_write(
                &pending.stream_id,
                pending.seq,
                Err("input acknowledgement worker stopped".into()),
            );
        }
    }

    /// Applies a write outcome and notifies waiters outside the lock.
    pub(crate) fn complete_write(&self, stream_id: &str, seq: u64, result: Result<(), String>) {
        complete_and_notify(&self.table, stream_id, seq, result);
    }
}

fn lock_table(table: &Mutex<InputStreamTable>) -> MutexGuard<'_, InputStreamTable> {
    table
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn complete_and_notify(
    table: &Mutex<InputStreamTable>,
    stream_id: &str,
    seq: u64,
    result: Result<(), String>,
) {
    let notify = lock_table(table).complete_write(stream_id, seq, result);
    for (waiter, result) in notify {
        waiter(result);
    }
}

fn run_completions(table: &Mutex<InputStreamTable>, receiver: mpsc::Receiver<PendingCompletion>) {
    for pending in receiver {
        let result = match pending.completion.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err("terminal input writer closed before the write".into()),
        };
        complete_and_notify(table, &pending.stream_id, pending.seq, result);
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HandoffInputStreams {
    pub incarnation: String,
    #[serde(default)]
    pub streams: Vec<HandoffInputStream>,
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HandoffInputStream {
    pub stream_id: String,
    pub last_admitted_seq: u64,
    pub last_written_seq: u64,
    /// Unpadded base64 of the retained digest keys, oldest first; the last one
    /// belongs to `last_admitted_seq`.
    #[serde(default)]
    pub digests: String,
    /// Retained sequences whose write failed; every other one was written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<u64>,
}

/// SHA-256 over the request's text and key strings exactly as sent, with
/// length prefixes so no two payloads share an encoding.
pub(crate) fn input_digest(text: &str, keys: &[String]) -> InputDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"herdr.terminal.input.v1\0");
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
    hasher.update((keys.len() as u64).to_le_bytes());
    for key in keys {
        hasher.update((key.len() as u64).to_le_bytes());
        hasher.update(key.as_bytes());
    }
    finish_input_digest(hasher)
}

/// Separate domain preserves the published encoded-input digest and prevents
/// a sequence from changing between encoded and literal writes on replay.
pub(crate) fn literal_input_digest(literal: &str) -> InputDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"herdr.terminal.input.literal.v1\0");
    hasher.update((literal.len() as u64).to_le_bytes());
    hasher.update(literal.as_bytes());
    finish_input_digest(hasher)
}

fn finish_input_digest(hasher: Sha256) -> InputDigest {
    let full = hasher.finalize();
    let mut key = DigestKey::default();
    key.copy_from_slice(&full[..DIGEST_KEY_LEN]);
    InputDigest {
        hex: format!("{full:x}"),
        key,
    }
}

/// Unpredictable, never-repeating identifier. The standard library seeds each
/// `RandomState` from the operating system's random source; the process id,
/// clock and counter keep tokens distinct even if that seed repeats.
fn mint_token(prefix: &str) -> String {
    use std::hash::{BuildHasher, Hasher};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    hasher.update(counter.to_le_bytes());
    for round in 0..2u64 {
        let mut random = std::collections::hash_map::RandomState::new().build_hasher();
        random.write_u64(counter ^ round);
        hasher.update(random.finish().to_le_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("{prefix}{}", &digest[..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn d(payload: &str) -> DigestKey {
        input_digest(payload, &[]).key
    }

    fn recorder() -> (WriteWaiter, mpsc::Receiver<Result<(), String>>) {
        let (tx, rx) = mpsc::channel();
        (
            Box::new(move |result| {
                let _ = tx.send(result);
            }),
            rx,
        )
    }

    fn admit(
        table: &mut InputStreamTable,
        stream: &str,
        seq: u64,
        digest: &str,
        now: Instant,
    ) -> mpsc::Receiver<Result<(), String>> {
        assert_eq!(
            table.classify_write(stream, seq, &d(digest), now),
            Ok(WriteClass::Next)
        );
        let (waiter, rx) = recorder();
        table.record_admitted(stream, seq, d(digest), waiter);
        rx
    }

    fn notify(pending: Vec<(WriteWaiter, Result<(), String>)>) {
        for (waiter, result) in pending {
            waiter(result);
        }
    }

    #[test]
    fn digest_distinguishes_text_keys_and_boundaries() {
        let enter = vec!["Enter".to_string()];
        assert_eq!(input_digest("ls", &enter), input_digest("ls", &enter));
        assert_ne!(input_digest("ls", &enter), input_digest("ls", &[]));
        assert_ne!(
            input_digest("ls", &enter),
            input_digest("ls", &["enter".into()])
        );
        assert_ne!(
            input_digest("ab", &["c".into()]),
            input_digest("a", &["bc".into()])
        );
        assert_ne!(input_digest("é", &[]), input_digest("e\u{301}", &[]));
    }

    #[test]
    fn tokens_are_prefixed_and_unique() {
        let a = mint_token("inc_");
        let b = mint_token("inc_");
        assert!(a.starts_with("inc_") && a.len() == 36);
        assert_ne!(a, b);
        assert_ne!(
            InputStreamTable::new().incarnation(),
            InputStreamTable::new().incarnation()
        );
    }

    #[test]
    fn stale_incarnation_is_refused() {
        let table = InputStreamTable::new();
        assert_eq!(table.check_incarnation(None), Ok(()));
        assert_eq!(table.check_incarnation(Some(table.incarnation())), Ok(()));
        assert_eq!(
            table.check_incarnation(Some("inc_old")),
            Err(InputStreamError::StaleTarget {
                current: table.incarnation().to_owned()
            })
        );
    }

    #[test]
    fn sequences_must_start_at_one_and_stay_contiguous() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        assert_eq!(
            table.classify_write(&stream, 2, &d("d"), now),
            Err(InputStreamError::SequenceGap {
                last_admitted_seq: 0
            })
        );
        assert_eq!(
            table.classify_write(&stream, 0, &d("d"), now),
            Err(InputStreamError::SequenceGap {
                last_admitted_seq: 0
            })
        );
        let _ = admit(&mut table, &stream, 1, "d1", now);
        assert_eq!(
            table.classify_write(&stream, 3, &d("d3"), now),
            Err(InputStreamError::SequenceGap {
                last_admitted_seq: 1
            })
        );
        assert_eq!(
            table.classify_write("tis_unknown", 1, &d("d"), now),
            Err(InputStreamError::StreamNotFound)
        );
    }

    #[test]
    fn resend_of_written_sequence_is_duplicate_and_mismatch_is_refused() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let rx = admit(&mut table, &stream, 1, "enter", now);
        assert_eq!(
            table.classify_write(&stream, 1, &d("enter"), now),
            Ok(WriteClass::DuplicatePending)
        );
        notify(table.complete_write(&stream, 1, Ok(())));
        assert_eq!(rx.try_recv(), Ok(Ok(())));
        assert_eq!(
            table.classify_write(&stream, 1, &d("enter"), now),
            Ok(WriteClass::DuplicateWritten)
        );
        assert_eq!(
            table.classify_write(&stream, 1, &d("other"), now),
            Err(InputStreamError::PayloadMismatch)
        );
        let position = table.position(&stream, now).unwrap();
        assert_eq!(
            (position.last_admitted_seq, position.last_written_seq),
            (1, 1)
        );
    }

    #[test]
    fn pending_resend_waits_for_the_original_write() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let original = admit(&mut table, &stream, 1, "d", now);
        let (waiter, resend) = recorder();
        table.wait_for(&stream, 1, waiter);
        assert!(resend.try_recv().is_err());
        let position = table.position(&stream, now).unwrap();
        assert_eq!(
            (position.last_admitted_seq, position.last_written_seq),
            (1, 0)
        );

        notify(table.complete_write(&stream, 1, Ok(())));
        assert_eq!(original.try_recv(), Ok(Ok(())));
        assert_eq!(resend.try_recv(), Ok(Ok(())));
    }

    #[test]
    fn later_completion_proves_earlier_sequences_of_the_stream() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let first = admit(&mut table, &stream, 1, "a", now);
        let second = admit(&mut table, &stream, 2, "b", now);
        notify(table.complete_write(&stream, 2, Ok(())));
        assert_eq!(first.try_recv(), Ok(Ok(())));
        assert_eq!(second.try_recv(), Ok(Ok(())));
        notify(table.complete_write(&stream, 1, Ok(())));
        assert_eq!(table.position(&stream, now).unwrap().last_written_seq, 2);
    }

    #[test]
    fn failed_write_is_reported_and_not_counted_as_written() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let rx = admit(&mut table, &stream, 1, "d", now);
        notify(table.complete_write(&stream, 1, Err("pty closed".into())));
        assert_eq!(rx.try_recv(), Ok(Err("pty closed".into())));
        assert_eq!(table.position(&stream, now).unwrap().last_written_seq, 0);
        assert_eq!(
            table.classify_write(&stream, 1, &d("d"), now),
            Err(InputStreamError::WriteFailed("pty closed".into()))
        );
    }

    #[test]
    fn digests_are_retained_for_a_bounded_window() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let total = RETAINED_SEQUENCES_PER_STREAM as u64 + 5;
        for seq in 1..=total {
            let _ = admit(&mut table, &stream, seq, &format!("d{seq}"), now);
            notify(table.complete_write(&stream, seq, Ok(())));
        }
        assert_eq!(
            table.classify_write(&stream, 5, &d("d5"), now),
            Err(InputStreamError::SequenceNotRetained)
        );
        assert_eq!(
            table.classify_write(&stream, 6, &d("d6"), now),
            Ok(WriteClass::DuplicateWritten)
        );
    }

    /// Writes queued while the program is not reading pile up past the
    /// window; their digests stay until they complete, then the window is
    /// back to its limit without another write.
    fn backlog_past_the_window(table: &mut InputStreamTable, now: Instant) -> (String, u64) {
        let stream = table.open(now).unwrap().stream_id;
        let total = RETAINED_SEQUENCES_PER_STREAM as u64 + 72;
        for seq in 1..=total {
            let _ = admit(table, &stream, seq, &format!("d{seq}"), now);
        }
        (stream, total)
    }

    #[test]
    fn pending_sequences_are_never_dropped_from_the_window() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let (stream, _) = backlog_past_the_window(&mut table, now);
        assert_eq!(
            table.classify_write(&stream, 1, &d("d1"), now),
            Ok(WriteClass::DuplicatePending)
        );
    }

    #[test]
    fn completing_a_backlog_trims_the_window_to_its_limit() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let (stream, total) = backlog_past_the_window(&mut table, now);
        notify(table.complete_write(&stream, total, Ok(())));
        let first_kept = total - RETAINED_SEQUENCES_PER_STREAM as u64 + 1;
        assert_eq!(
            table.classify_write(
                &stream,
                first_kept - 1,
                &d(&format!("d{}", first_kept - 1)),
                now
            ),
            Err(InputStreamError::SequenceNotRetained)
        );
        assert_eq!(
            table.classify_write(&stream, first_kept, &d(&format!("d{first_kept}")), now),
            Ok(WriteClass::DuplicateWritten)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_drained_backlog_survives_handoff_with_its_identity() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let (stream, total) = backlog_past_the_window(&mut table, now);
        let quiet = table.open(now).unwrap().stream_id;

        // The handoff drains the writer without reporting completions first.
        let exported = table.export_for_handoff();
        let json = serde_json::to_string(&exported).unwrap();
        let mut imported =
            InputStreamTable::from_handoff(serde_json::from_str(&json).unwrap(), now);

        assert_eq!(imported.incarnation(), table.incarnation());
        let position = imported.position(&stream, now).unwrap();
        assert_eq!(
            (position.last_admitted_seq, position.last_written_seq),
            (total, total)
        );
        assert!(imported.position(&quiet, now).is_ok());
        let first_kept = total - HANDOFF_RETAINED_SEQUENCES_PER_STREAM as u64 + 1;
        assert_eq!(
            imported.classify_write(&stream, total, &d(&format!("d{total}")), now),
            Ok(WriteClass::DuplicateWritten)
        );
        assert_eq!(
            imported.classify_write(&stream, first_kept, &d(&format!("d{first_kept}")), now),
            Ok(WriteClass::DuplicateWritten)
        );
        assert_eq!(
            imported.classify_write(&stream, first_kept - 1, &d("anything"), now),
            Err(InputStreamError::SequenceNotRetained)
        );
        assert_eq!(
            imported.classify_write(&stream, total + 1, &d("next"), now),
            Ok(WriteClass::Next)
        );
    }

    #[test]
    fn idle_streams_expire_but_streams_with_pending_writes_do_not() {
        let start = Instant::now();
        let mut table = InputStreamTable::new();
        let idle = table.open(start).unwrap().stream_id;
        let busy = table.open(start).unwrap().stream_id;
        let _ = admit(&mut table, &busy, 1, "d", start);
        let later = start + STREAM_IDLE_EXPIRY;
        assert_eq!(
            table.position(&idle, later),
            Err(InputStreamError::StreamNotFound)
        );
        assert!(table.position(&busy, later).is_ok());
        assert_eq!(
            table.classify_write(&idle, 1, &d("d"), later),
            Err(InputStreamError::StreamNotFound)
        );
    }

    #[test]
    fn opening_past_the_cap_evicts_the_least_recently_used_idle_stream() {
        let start = Instant::now();
        let mut table = InputStreamTable::new();
        let mut ids = Vec::new();
        for offset in 0..MAX_STREAMS_PER_TERMINAL {
            ids.push(
                table
                    .open(start + Duration::from_secs(offset as u64))
                    .unwrap()
                    .stream_id,
            );
        }
        let _ = admit(&mut table, &ids[0], 1, "d", start);
        let now = start + Duration::from_secs(100);
        table.open(now).unwrap();
        assert!(table.position(&ids[0], now).is_ok(), "pending stream kept");
        assert_eq!(
            table.position(&ids[1], now),
            Err(InputStreamError::StreamNotFound)
        );
    }

    #[test]
    fn opening_fails_when_every_stream_has_a_write_in_progress() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        for _ in 0..MAX_STREAMS_PER_TERMINAL {
            let stream = table.open(now).unwrap().stream_id;
            let _ = admit(&mut table, &stream, 1, "d", now);
        }
        assert_eq!(table.open(now), Err(InputStreamError::TooManyStreams));
    }

    #[test]
    fn streams_are_independent() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let a = table.open(now).unwrap().stream_id;
        let b = table.open(now).unwrap().stream_id;
        assert_ne!(a, b);
        let _ = admit(&mut table, &a, 1, "a1", now);
        let _ = admit(&mut table, &b, 1, "b1", now);
        notify(table.complete_write(&a, 1, Ok(())));
        assert_eq!(table.position(&a, now).unwrap().last_written_seq, 1);
        assert_eq!(table.position(&b, now).unwrap().last_written_seq, 0);
    }

    #[cfg(unix)]
    #[test]
    fn handoff_round_trip_keeps_incarnation_and_marks_drained_writes_written() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let _ = admit(&mut table, &stream, 1, "a", now);
        notify(table.complete_write(&stream, 1, Ok(())));
        let _ = admit(&mut table, &stream, 2, "b", now);

        let exported = table.export_for_handoff();
        let json = serde_json::to_string(&exported).unwrap();
        let mut imported =
            InputStreamTable::from_handoff(serde_json::from_str(&json).unwrap(), now);

        assert_eq!(imported.incarnation(), table.incarnation());
        let position = imported.position(&stream, now).unwrap();
        assert_eq!(
            (position.last_admitted_seq, position.last_written_seq),
            (2, 2)
        );
        assert_eq!(
            imported.classify_write(&stream, 2, &d("b"), now),
            Ok(WriteClass::DuplicateWritten)
        );
        assert_eq!(
            imported.classify_write(&stream, 2, &d("x"), now),
            Err(InputStreamError::PayloadMismatch)
        );
        assert_eq!(
            imported.classify_write(&stream, 3, &d("c"), now),
            Ok(WriteClass::Next)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_malformed_stream_is_dropped_alone() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let good = table.open(now).unwrap().stream_id;
        let _ = admit(&mut table, &good, 1, "a", now);
        let mut state = table.export_for_handoff();
        let valid = state.streams[0].clone();
        let malformed = [
            HandoffInputStream {
                stream_id: "tis_written_past_admitted".into(),
                last_admitted_seq: 2,
                last_written_seq: 3,
                digests: String::new(),
                failed: Vec::new(),
            },
            HandoffInputStream {
                stream_id: "tis_more_digests_than_sequences".into(),
                ..valid.clone()
            }
            .with_digests(2),
            HandoffInputStream {
                stream_id: "tis_failed_outside_window".into(),
                failed: vec![7],
                ..valid.clone()
            },
            HandoffInputStream {
                stream_id: "tis_not_base64".into(),
                digests: "!!".into(),
                ..valid.clone()
            },
            HandoffInputStream {
                stream_id: "tis_twice".into(),
                ..valid.clone()
            },
            HandoffInputStream {
                stream_id: "tis_twice".into(),
                ..valid.clone()
            },
        ];
        state.streams.extend(malformed);

        let mut imported = InputStreamTable::from_handoff(state, now);
        assert_eq!(imported.incarnation(), table.incarnation());
        assert_eq!(imported.streams.len(), 1);
        assert_eq!(
            imported.classify_write(&good, 1, &d("a"), now),
            Ok(WriteClass::DuplicateWritten)
        );
    }

    #[cfg(unix)]
    impl HandoffInputStream {
        fn with_digests(mut self, count: usize) -> Self {
            use base64::Engine;
            self.digests = base64::engine::general_purpose::STANDARD_NO_PAD
                .encode(vec![0u8; count * DIGEST_KEY_LEN]);
            self
        }
    }

    #[cfg(unix)]
    #[test]
    fn handoff_state_without_incarnation_mints_a_new_one() {
        let state = HandoffInputStreams {
            incarnation: String::new(),
            streams: Vec::new(),
        };
        assert!(InputStreamTable::from_handoff(state, Instant::now())
            .incarnation()
            .starts_with("inc_"));
    }

    /// Every terminal at its limits: the most streams, each with a full
    /// window. The handoff line is capped at 16 MiB and also carries each
    /// pane's history, so stream state must stay a small part of it.
    #[cfg(unix)]
    #[test]
    fn handoff_state_stays_small_at_the_limits() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        for _ in 0..MAX_STREAMS_PER_TERMINAL {
            let stream = table.open(now).unwrap().stream_id;
            for seq in 1..=RETAINED_SEQUENCES_PER_STREAM as u64 + 1000 {
                let _ = admit(&mut table, &stream, seq, &format!("{seq}"), now);
                notify(table.complete_write(&stream, seq, Ok(())));
            }
        }
        let terminals = 256;
        let per_terminal = serde_json::to_string(&table.export_for_handoff())
            .unwrap()
            .len();
        assert!(
            per_terminal < 32 * 1024,
            "{per_terminal} bytes per terminal"
        );
        assert!(
            per_terminal * terminals < 16 * 1024 * 1024 / 2,
            "{} bytes for {terminals} terminals",
            per_terminal * terminals
        );
    }

    #[test]
    fn the_completion_worker_is_shared_and_stops_with_its_terminal() {
        let streams = TerminalInputStreams::new();
        let stream = streams.lock().open(Instant::now()).unwrap().stream_id;
        let mut replies = Vec::new();
        for seq in 1..=3 {
            let sender = streams.completion_sender().unwrap();
            let (waiter, rx) = recorder();
            streams.lock().record_admitted(&stream, seq, d("x"), waiter);
            let (done_tx, completion) = mpsc::channel();
            done_tx.send(Ok(())).unwrap();
            streams.track(
                &sender,
                PendingCompletion {
                    stream_id: stream.clone(),
                    seq,
                    completion,
                },
            );
            replies.push(rx);
        }
        for rx in replies {
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(Ok(())));
        }

        let table = Arc::downgrade(&streams.table);
        drop(streams);
        let deadline = Instant::now() + Duration::from_secs(5);
        while table.upgrade().is_some() {
            assert!(Instant::now() < deadline, "completion worker kept running");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
