use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Opaque identity for a server-owned terminal.
///
/// During the pane-backed transition this is stored one-to-one beside panes,
/// but callers must not derive it from a pane id or layout position.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TerminalId(String);

static NEXT_TERMINAL_ID: AtomicU64 = AtomicU64::new(1);
static IMPORTED_TERMINAL_IDS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn imported_ids() -> &'static Mutex<HashSet<String>> {
    IMPORTED_TERMINAL_IDS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn reserve_ids(reserved: &mut HashSet<String>, ids: &[TerminalId]) -> io::Result<()> {
    let mut incoming = HashSet::new();
    for id in ids {
        if id.0.is_empty() || !incoming.insert(id.0.clone()) || reserved.contains(&id.0) {
            return Err(io::Error::other(
                "empty, duplicate or already reserved imported terminal ID",
            ));
        }
    }
    reserved.extend(incoming);
    Ok(())
}

fn allocate_unreserved(
    micros: u128,
    reserved: &HashSet<String>,
    mut next_counter: impl FnMut() -> u64,
) -> TerminalId {
    loop {
        let id = format!("term_{micros:x}{:x}", next_counter());
        if !reserved.contains(&id) {
            return TerminalId(id);
        }
    }
}

impl TerminalId {
    pub fn alloc() -> Self {
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_micros())
            .unwrap_or(0);
        let reserved = imported_ids()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        allocate_unreserved(micros, &reserved, || {
            match NEXT_TERMINAL_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            }) {
                Ok(counter) => counter,
                // The infallible allocation API must never wrap into a reused
                // identity. Exhaustion is a process invariant failure.
                Err(_) => panic!("terminal identity counter exhausted"),
            }
        })
    }

    pub(crate) fn checked_import(value: String) -> io::Result<Self> {
        if value.is_empty() {
            return Err(io::Error::other("imported terminal ID is empty"));
        }
        Ok(Self(value))
    }

    /// Reserve the entire admitted set before App construction. Reservations
    /// deliberately outlive terminals so stale clients cannot name a new one.
    pub(crate) fn reserve_imported(ids: &[Self]) -> io::Result<()> {
        let mut reserved = imported_ids()
            .lock()
            .map_err(|_| io::Error::other("terminal identity reservation lock poisoned"))?;
        reserve_ids(&mut reserved, ids)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Borrow<str> for TerminalId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn import_reservation_is_atomic_and_keeps_opaque_bytes() {
        let ids = vec![
            TerminalId::checked_import("opaque: old identity".into()).unwrap(),
            TerminalId::checked_import("term_1234".into()).unwrap(),
        ];
        let mut reserved = HashSet::new();
        reserve_ids(&mut reserved, &ids).unwrap();
        assert!(reserved.contains("opaque: old identity"));
        let before = reserved.clone();
        let duplicate = vec![
            TerminalId::checked_import("new".into()).unwrap(),
            ids[0].clone(),
        ];
        assert!(reserve_ids(&mut reserved, &duplicate).is_err());
        assert_eq!(reserved, before);
        assert!(TerminalId::checked_import(String::new()).is_err());
        assert!(reserve_ids(&mut reserved, &[ids[0].clone(), ids[0].clone()]).is_err());
    }

    #[test]
    fn allocation_skips_imported_ids_even_with_ambiguous_suffix_and_reversing_clock() {
        let old = TerminalId::checked_import("term_1234".into()).unwrap();
        let mut reserved = HashSet::new();
        reserve_ids(&mut reserved, &[old.clone()]).unwrap();
        let mut counters = [4, 5].into_iter();
        assert_eq!(
            allocate_unreserved(0x123, &reserved, || counters.next().unwrap()).as_str(),
            "term_1235"
        );
        let mut counters = [0x34, 0x35].into_iter();
        assert_eq!(
            allocate_unreserved(0x12, &reserved, || counters.next().unwrap()).as_str(),
            "term_1235"
        );
        assert_eq!(allocate_unreserved(0, &reserved, || 1).as_str(), "term_01");
        drop(old);
        assert!(
            reserved.contains("term_1234"),
            "closing an imported identity must not free its reservation"
        );
    }

    #[test]
    fn terminal_ids_support_borrowed_lookup_without_changing_identity() {
        let id = TerminalId::alloc();
        let serialized = serde_json::to_string(&id).unwrap();
        let restored: TerminalId = serde_json::from_str(&serialized).unwrap();
        let mut terminals = HashMap::from([(id.clone(), 1)]);

        assert_eq!(terminals.get(restored.as_str()), Some(&1));
        assert_eq!(terminals.get("missing-terminal"), None);
        *terminals.get_mut(restored.as_str()).unwrap() = 2;
        assert_eq!(terminals.remove(id.as_str()), Some(2));
        assert!(terminals.is_empty());
    }
}
