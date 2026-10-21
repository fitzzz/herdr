//! Acknowledged, de-duplicated input streams for one terminal incarnation.
//!
//! Backs the `terminal.input.*` API. Each live terminal runtime owns one
//! table, so stream state ends with the runtime and a recreated terminal gets
//! a new incarnation with no streams. The table is pure data: callers hold its
//! lock across admission, de-duplication and queueing the bytes for the PTY,
//! and report the PTY write outcome back through [`InputStreamTable::complete_write`].

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

pub(crate) const MAX_STREAMS_PER_TERMINAL: usize = 32;
pub(crate) const RETAINED_SEQUENCES_PER_STREAM: usize = 128;
pub(crate) const STREAM_IDLE_EXPIRY: Duration = Duration::from_secs(24 * 60 * 60);

/// Called once with the PTY write outcome of one admitted sequence.
pub(crate) type WriteWaiter = Box<dyn FnOnce(Result<(), String>) + Send>;

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
    digest: String,
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
        digest: &str,
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
        if entry.digest != digest {
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
        digest: String,
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
        while stream.retained.len() > RETAINED_SEQUENCES_PER_STREAM
            && stream
                .retained
                .front()
                .is_some_and(|entry| !matches!(entry.outcome, SequenceOutcome::Pending(_)))
        {
            stream.retained.pop_front();
        }
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
    /// marked pending here were written and are exported as written.
    #[cfg(unix)]
    pub(crate) fn export_for_handoff(&self) -> HandoffInputStreams {
        HandoffInputStreams {
            incarnation: self.incarnation.clone(),
            streams: self
                .streams
                .iter()
                .map(|(stream_id, stream)| {
                    let mut last_written_seq = stream.last_written_seq;
                    let retained = stream
                        .retained
                        .iter()
                        .map(|entry| {
                            let written = !matches!(entry.outcome, SequenceOutcome::Failed(_));
                            if written {
                                last_written_seq = last_written_seq.max(entry.seq);
                            }
                            HandoffInputSequence {
                                seq: entry.seq,
                                digest: entry.digest.clone(),
                                written,
                            }
                        })
                        .collect();
                    HandoffInputStream {
                        stream_id: stream_id.clone(),
                        last_admitted_seq: stream.last_admitted_seq,
                        last_written_seq,
                        retained,
                    }
                })
                .collect(),
        }
    }

    /// Restores exported state; malformed state yields a fresh incarnation so a
    /// client can never resume against sequence numbers it cannot trust.
    #[cfg(unix)]
    pub(crate) fn from_handoff(state: HandoffInputStreams, now: Instant) -> Self {
        let mut streams = HashMap::new();
        let valid = !state.incarnation.is_empty()
            && state.streams.len() <= MAX_STREAMS_PER_TERMINAL
            && state.streams.into_iter().all(|stream| {
                let contiguous = stream
                    .retained
                    .iter()
                    .zip(stream.retained.iter().skip(1))
                    .all(|(a, b)| a.seq + 1 == b.seq);
                let ends_at_admitted = stream
                    .retained
                    .last()
                    .is_none_or(|entry| entry.seq == stream.last_admitted_seq);
                let ok = !stream.stream_id.is_empty()
                    && stream.last_written_seq <= stream.last_admitted_seq
                    && stream.retained.len() <= RETAINED_SEQUENCES_PER_STREAM
                    && contiguous
                    && ends_at_admitted
                    && !streams.contains_key(&stream.stream_id);
                if ok {
                    streams.insert(
                        stream.stream_id,
                        StreamState {
                            last_admitted_seq: stream.last_admitted_seq,
                            last_written_seq: stream.last_written_seq,
                            last_used: now,
                            retained: stream
                                .retained
                                .into_iter()
                                .map(|entry| RetainedSequence {
                                    seq: entry.seq,
                                    digest: entry.digest,
                                    outcome: if entry.written {
                                        SequenceOutcome::Written
                                    } else {
                                        SequenceOutcome::Failed(
                                            "write failed before live handoff".into(),
                                        )
                                    },
                                })
                                .collect(),
                        },
                    );
                }
                ok
            });
        if !valid {
            tracing::warn!("discarding malformed terminal input stream handoff state");
            return Self::new();
        }
        Self {
            incarnation: state.incarnation,
            streams,
        }
    }
}

/// Shared handle: the API thread admits writes and PTY completion threads
/// report outcomes through the same lock.
#[derive(Clone)]
pub(crate) struct TerminalInputStreams(Arc<Mutex<InputStreamTable>>);

impl TerminalInputStreams {
    pub(crate) fn new() -> Self {
        Self::from_table(InputStreamTable::new())
    }

    pub(crate) fn from_table(table: InputStreamTable) -> Self {
        Self(Arc::new(Mutex::new(table)))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, InputStreamTable> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Applies a write outcome and notifies waiters outside the lock.
    pub(crate) fn complete_write(&self, stream_id: &str, seq: u64, result: Result<(), String>) {
        let notify = self.lock().complete_write(stream_id, seq, result);
        for (waiter, result) in notify {
            waiter(result);
        }
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
    #[serde(default)]
    pub retained: Vec<HandoffInputSequence>,
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HandoffInputSequence {
    pub seq: u64,
    pub digest: String,
    pub written: bool,
}

/// SHA-256 over the request's text and key strings exactly as sent, with
/// length prefixes so no two payloads share an encoding.
pub(crate) fn input_digest(text: &str, keys: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"herdr.terminal.input.v1\0");
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
    hasher.update((keys.len() as u64).to_le_bytes());
    for key in keys {
        hasher.update((key.len() as u64).to_le_bytes());
        hasher.update(key.as_bytes());
    }
    format!("{:x}", hasher.finalize())
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
            table.classify_write(stream, seq, digest, now),
            Ok(WriteClass::Next)
        );
        let (waiter, rx) = recorder();
        table.record_admitted(stream, seq, digest.into(), waiter);
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
            table.classify_write(&stream, 2, "d", now),
            Err(InputStreamError::SequenceGap {
                last_admitted_seq: 0
            })
        );
        assert_eq!(
            table.classify_write(&stream, 0, "d", now),
            Err(InputStreamError::SequenceGap {
                last_admitted_seq: 0
            })
        );
        let _ = admit(&mut table, &stream, 1, "d1", now);
        assert_eq!(
            table.classify_write(&stream, 3, "d3", now),
            Err(InputStreamError::SequenceGap {
                last_admitted_seq: 1
            })
        );
        assert_eq!(
            table.classify_write("tis_unknown", 1, "d", now),
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
            table.classify_write(&stream, 1, "enter", now),
            Ok(WriteClass::DuplicatePending)
        );
        notify(table.complete_write(&stream, 1, Ok(())));
        assert_eq!(rx.try_recv(), Ok(Ok(())));
        assert_eq!(
            table.classify_write(&stream, 1, "enter", now),
            Ok(WriteClass::DuplicateWritten)
        );
        assert_eq!(
            table.classify_write(&stream, 1, "other", now),
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
            table.classify_write(&stream, 1, "d", now),
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
            table.classify_write(&stream, 5, "d5", now),
            Err(InputStreamError::SequenceNotRetained)
        );
        assert_eq!(
            table.classify_write(&stream, 6, "d6", now),
            Ok(WriteClass::DuplicateWritten)
        );
    }

    #[test]
    fn pending_sequences_are_never_dropped_from_the_window() {
        let now = Instant::now();
        let mut table = InputStreamTable::new();
        let stream = table.open(now).unwrap().stream_id;
        let total = RETAINED_SEQUENCES_PER_STREAM as u64 + 3;
        for seq in 1..=total {
            let _ = admit(&mut table, &stream, seq, &format!("d{seq}"), now);
        }
        assert_eq!(
            table.classify_write(&stream, 1, "d1", now),
            Ok(WriteClass::DuplicatePending)
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
            table.classify_write(&idle, 1, "d", later),
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
            imported.classify_write(&stream, 2, "b", now),
            Ok(WriteClass::DuplicateWritten)
        );
        assert_eq!(
            imported.classify_write(&stream, 2, "x", now),
            Err(InputStreamError::PayloadMismatch)
        );
        assert_eq!(
            imported.classify_write(&stream, 3, "c", now),
            Ok(WriteClass::Next)
        );
    }

    #[cfg(unix)]
    #[test]
    fn malformed_handoff_state_mints_a_new_incarnation() {
        let now = Instant::now();
        let state = HandoffInputStreams {
            incarnation: "inc_old".into(),
            streams: vec![HandoffInputStream {
                stream_id: "tis_a".into(),
                last_admitted_seq: 2,
                last_written_seq: 3,
                retained: Vec::new(),
            }],
        };
        let table = InputStreamTable::from_handoff(state, now);
        assert_ne!(table.incarnation(), "inc_old");
    }
}
