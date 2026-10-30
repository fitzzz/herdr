//! Explicit first-hop terminal identity admission for the pinned old exporter.
//! Its ordered runtime/FD construction is trusted. This receipt proves identity
//! association; it does not prove frozen completion observations.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::handoff::HandoffManifest;
use crate::platform::{MigrationFileStamp, MigrationProcessStamp};
use crate::terminal::TerminalId;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CapturedPane {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub agent_name: Option<String>,
    pub agent_session: Option<Value>,
    pub child: Option<MigrationProcessStamp>,
    pub unavailable: Option<crate::handoff_runtime::UnavailableHandoffState>,
}

pub(crate) enum AdmittedPaneIdentity {
    Runtime(TerminalId),
    Unavailable(crate::handoff_runtime::ImportedHandoffPlaceholder),
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct IdentityReceipt {
    pub version: u32,
    pub operation_id: String,
    pub observation_epoch: String,
    pub source: MigrationProcessStamp,
    pub source_sha256: String,
    pub source_version: String,
    pub source_protocol: u32,
    pub candidate_sha256: String,
    pub candidate_version: String,
    pub candidate_protocol: u32,
    pub session: Option<String>,
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub data_dir_stamp: MigrationFileStamp,
    pub api_socket: PathBuf,
    pub api_socket_stamp: MigrationFileStamp,
    pub client_socket: PathBuf,
    pub client_socket_stamp: MigrationFileStamp,
    pub panes: Vec<CapturedPane>,
    /// Evidence from the capture. It is never treated as the frozen S/C state.
    pub captured_agents: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct IdentityClaim {
    operation_id: String,
    receipt_sha256: String,
    receipt_stamp: MigrationFileStamp,
    directory_stamp: MigrationFileStamp,
    importer: MigrationProcessStamp,
    source: MigrationProcessStamp,
    candidate_sha256: String,
    handoff_socket: PathBuf,
    handoff_socket_stamp: MigrationFileStamp,
    token: String,
}

pub(crate) struct IdentityAttempt {
    pub receipt: IdentityReceipt,
    receipt_sha256: String,
    claim: IdentityClaim,
    directory: File,
}

fn mismatch(message: &str) -> io::Error {
    io::Error::other(message)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_bytes(file: &mut File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(mismatch("migration receipt exceeds size limit"));
    }
    Ok(bytes)
}

impl IdentityAttempt {
    pub(crate) fn from_fds(fds: [i32; 3], socket: &Path, token: &str) -> io::Result<Self> {
        let [mut receipt_file, mut claim_file, directory] =
            crate::platform::migration_attempt_files(fds)?;
        let bytes = file_bytes(&mut receipt_file)?;
        let receipt_sha256 = digest(&bytes);
        let receipt: IdentityReceipt =
            serde_json::from_value(parse_unique_json(&bytes)?).map_err(io::Error::other)?;
        let claim: IdentityClaim =
            serde_json::from_value(parse_unique_json(&file_bytes(&mut claim_file)?)?)
                .map_err(io::Error::other)?;
        let own = crate::platform::migration_process_stamp(std::process::id())?;
        if receipt.version != 1
            || receipt.operation_id.is_empty()
            || receipt.observation_epoch != receipt.operation_id
            || claim.operation_id != receipt.operation_id
            || claim.receipt_sha256 != receipt_sha256
            || claim.receipt_stamp != crate::platform::migration_file_stamp(&receipt_file)?
            || claim.directory_stamp != crate::platform::migration_file_stamp(&directory)?
            || claim.importer != own
            || claim.source != receipt.source
            || own.uid != receipt.source.uid
            || own.boot_id != receipt.source.boot_id
            || own.pid_namespace != receipt.source.pid_namespace
            || own.mount_namespace != receipt.source.mount_namespace
            || crate::platform::migration_parent_pid() != receipt.source.pid
            || claim.candidate_sha256 != receipt.candidate_sha256
            || crate::platform::migration_executable_digest(own.pid)? != receipt.candidate_sha256
            || claim.handoff_socket != socket
            || claim.token != token
            || token.is_empty()
            || socket
                != receipt
                    .data_dir
                    .join(format!("herdr-handoff-{}.sock", receipt.source.pid))
        {
            return Err(mismatch("migration receipt/claim/importer binding differs"));
        }
        let attempt = Self {
            receipt,
            receipt_sha256,
            claim,
            directory,
        };
        attempt.verify_context()?;
        Ok(attempt)
    }

    fn verify_context(&self) -> io::Result<()> {
        let receipt = &self.receipt;
        if crate::platform::migration_process_stamp(receipt.source.pid)? != receipt.source
            || crate::platform::migration_executable_digest(receipt.source.pid)?
                != receipt.source_sha256
            || crate::session::active_name() != receipt.session
            || crate::config::config_path() != receipt.config_path
            || crate::session::data_dir() != receipt.data_dir
            || crate::api::socket_path() != receipt.api_socket
            || crate::server::socket_paths::client_socket_path() != receipt.client_socket
            || crate::platform::migration_file_stamp(&File::open(&receipt.data_dir)?)?
                != receipt.data_dir_stamp
            || crate::platform::migration_socket_stamp(&receipt.api_socket)?
                != receipt.api_socket_stamp
            || crate::platform::migration_socket_stamp(&receipt.client_socket)?
                != receipt.client_socket_stamp
        {
            return Err(mismatch(
                "migration exporter/session/socket context differs",
            ));
        }
        Ok(())
    }

    pub(crate) fn verify_peer(&self, stream: &UnixStream) -> io::Result<()> {
        self.verify_context()?;
        if crate::platform::migration_peer_stamp(stream)? != self.receipt.source
            || crate::platform::migration_socket_stamp(&self.claim.handoff_socket)?
                != self.claim.handoff_socket_stamp
        {
            return Err(mismatch("migration handoff peer/socket differs"));
        }
        Ok(())
    }

    pub(crate) fn admit(
        &self,
        manifest: &HandoffManifest,
        bytes: &[u8],
    ) -> io::Result<HashMap<u32, AdmittedPaneIdentity>> {
        let identities = admit_manifest(&self.receipt, manifest, codex_command_line_resume)?;
        self.verify_processes()?;
        let record = serde_json::to_vec(&serde_json::json!({
            "operation_id": self.receipt.operation_id,
            "receipt_sha256": self.receipt_sha256,
            "manifest_sha256": digest(bytes),
            "importer": self.claim.importer,
            "source": self.receipt.source,
            "token": self.claim.token,
            "observation_epoch": self.receipt.observation_epoch,
            "ownership_transfer": "precommit"
        }))
        .map_err(io::Error::other)?;
        crate::platform::migration_create_record(&self.directory, "admitted.json", &record)?;
        Ok(identities)
    }

    pub(crate) fn verify_processes(&self) -> io::Result<()> {
        self.verify_context()?;
        for pane in &self.receipt.panes {
            if let Some(child) = &pane.child {
                if crate::platform::migration_process_stamp(child.pid)? != *child {
                    return Err(mismatch("migration child identity changed"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn record_error(&self, phase: &str, error: &io::Error) {
        if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({
            "operation_id": self.receipt.operation_id, "phase": phase,
            "error": error.to_string(), "ownership_transfer": "precommit",
            "handoff_request_sent": true, "clients_disconnected": true
        })) {
            let _ = crate::platform::migration_create_record(
                &self.directory,
                "import-error.json",
                &bytes,
            );
        }
    }
}

fn layout_ids(layout: &crate::persist::LayoutSnapshot, ids: &mut Vec<u32>) {
    match layout {
        crate::persist::LayoutSnapshot::Pane(id) => ids.push(*id),
        crate::persist::LayoutSnapshot::Split { first, second, .. } => {
            layout_ids(first, ids);
            layout_ids(second, ids);
        }
    }
}

/// The conversation the foreground Codex process of the pane whose shell is
/// `child_pid` resumes on its own command line, read the way detection reads
/// it.
fn codex_command_line_resume(child_pid: u32) -> Option<String> {
    let job = crate::detect::foreground_job(child_pid)?;
    let (agent, _, index) = crate::detect::identify_agent_process_in_job(&job)?;
    let argv = job.processes.get(index)?.argv.as_deref()?;
    (agent == crate::detect::Agent::Codex)
        .then(|| crate::agent_resume::resumed_session_from_process_argv(agent, argv))
        .flatten()
}

/// The conversation a source API names for a pane from its Codex's own
/// `resume <id>` command line (`TerminalState::codex_command_line_session`),
/// which a session save leaves out, while the pane's Codex still resumes it.
fn unsaved_codex_resume(
    child: Option<&MigrationProcessStamp>,
    codex_resume: impl Fn(u32) -> Option<String>,
) -> io::Result<Option<Value>> {
    let Some(value) = child.and_then(|child| codex_resume(child.pid)) else {
        return Ok(None);
    };
    serde_json::to_value(crate::api::schema::AgentSessionInfo {
        source: "herdr:codex".into(),
        agent: "codex".into(),
        kind: crate::agent_resume::AgentSessionRefKind::Id,
        value,
    })
    .map(Some)
    .map_err(io::Error::other)
}

/// Complete admission. Run before `validated` and before restoration can
/// allocate, resume, spawn, or publish a binding. Its only outside read is
/// `codex_resume`, the conversation a pane's Codex resumes on its command
/// line, given the pane's shell process id; it is read only for a captured
/// conversation the frozen snapshot does not save.
pub(crate) fn admit_manifest(
    receipt: &IdentityReceipt,
    manifest: &HandoffManifest,
    codex_resume: impl Fn(u32) -> Option<String>,
) -> io::Result<HashMap<u32, AdmittedPaneIdentity>> {
    super::handoff::validate_manifest_format(manifest)?;
    if receipt.version != 1
        || receipt.operation_id.is_empty()
        || receipt.observation_epoch != receipt.operation_id
        || manifest.source_version != receipt.source_version
        || manifest.source_protocol != receipt.source_protocol
        || manifest.expected_version.as_deref() != Some(receipt.candidate_version.as_str())
        || manifest.expected_protocol != Some(receipt.candidate_protocol)
        || !matches!(manifest.snapshot.version, 3 | 4)
    {
        return Err(mismatch(
            "migration source/candidate/snapshot version differs",
        ));
    }
    let mut captured = HashMap::new();
    let mut terminals = HashSet::new();
    let mut children = HashSet::new();
    for pane in &receipt.panes {
        if pane.pane_id.is_empty()
            || captured.insert(pane.pane_id.as_str(), pane).is_some()
            || !terminals.insert(pane.terminal_id.as_str())
        {
            return Err(mismatch(
                "migration capture identities are incomplete or duplicated",
            ));
        }
        TerminalId::checked_import(pane.terminal_id.clone())?;
        match (&pane.child, &pane.unavailable) {
            (Some(child), None)
                if child.pid != 0
                    && child.uid == receipt.source.uid
                    && child.boot_id == receipt.source.boot_id
                    && child.pid_namespace == receipt.source.pid_namespace
                    && child.mount_namespace == receipt.source.mount_namespace
                    && children.insert(child.pid) => {}
            (None, Some(state))
                if state.restore_error == crate::handoff_runtime::SAVED_DIRECTORY_UNAVAILABLE
                    && state.cwd.is_absolute()
                    && pane.agent_name.is_none()
                    && pane.agent_session.is_none() => {}
            _ => {
                return Err(mismatch(
                    "migration runtime presence/absence capture differs",
                ))
            }
        }
    }
    let mut runtime_by_raw = HashMap::new();
    for runtime in &manifest.panes {
        if runtime.child_pid == 0 || runtime_by_raw.insert(runtime.pane_id, runtime).is_some() {
            return Err(mismatch(
                "migration runtime identity is missing or duplicated",
            ));
        }
    }
    let mut identities = HashMap::new();
    let mut workspaces = HashSet::new();
    for workspace in &manifest.snapshot.workspaces {
        let workspace_id = workspace
            .id
            .as_deref()
            .ok_or_else(|| mismatch("migration workspace ID absent"))?;
        if crate::workspace::public_workspace_number(workspace_id).is_none_or(|number| number == 0)
            || !workspaces.insert(workspace_id)
            || workspace.public_tab_numbers.len() != workspace.tabs.len()
        {
            return Err(mismatch("migration workspace/tab identity differs"));
        }
        let mut tab_numbers = HashSet::new();
        let mut public_numbers = HashSet::new();
        let mut workspace_raw_ids = HashSet::new();
        for (tab, number) in workspace.tabs.iter().zip(&workspace.public_tab_numbers) {
            if *number == 0 || !tab_numbers.insert(*number) {
                return Err(mismatch("migration tab number is absent or duplicated"));
            }
            let tab_id = crate::workspace::public_tab_id_for_number(workspace_id, *number);
            let mut leaves = Vec::new();
            layout_ids(&tab.layout, &mut leaves);
            let leaf_set: HashSet<_> = leaves.iter().copied().collect();
            if leaf_set.len() != leaves.len()
                || leaf_set != tab.panes.keys().copied().collect()
                || tab.focused.is_some_and(|id| !leaf_set.contains(&id))
                || tab.root_pane.is_some_and(|id| !leaf_set.contains(&id))
            {
                return Err(mismatch("migration layout/pane/focus coverage differs"));
            }
            for raw in leaves {
                let number = workspace
                    .public_pane_numbers
                    .get(&raw)
                    .ok_or_else(|| mismatch("migration public pane number absent"))?;
                if *number == 0
                    || !public_numbers.insert(*number)
                    || !workspace_raw_ids.insert(raw)
                    || identities.contains_key(&raw)
                {
                    return Err(mismatch("migration raw/public pane identity duplicated"));
                }
                let pane_id = crate::workspace::public_pane_id_for_number(workspace_id, *number);
                let capture = captured
                    .get(pane_id.as_str())
                    .ok_or_else(|| mismatch("migration frozen pane is absent from capture"))?;
                let saved = &tab.panes[&raw];
                // The capture holds the conversation identity as the source API
                // publishes it; the snapshot also saves how the agent was
                // launched (options, account), which is not part of who
                // occupies the pane and still crosses the handoff in it.
                let native = saved
                    .agent_session
                    .as_ref()
                    .map(|session| {
                        serde_json::to_value(crate::api::schema::AgentSessionInfo {
                            source: session.source.clone(),
                            agent: session.agent.clone(),
                            kind: session.kind,
                            value: session.value.clone(),
                        })
                    })
                    .transpose()
                    .map_err(io::Error::other)?;
                // A Codex resumed by a command typed into the pane is named on
                // the API before its first turn reports it, but not saved.
                let occupant = if native.is_none() && capture.agent_session.is_some() {
                    unsaved_codex_resume(capture.child.as_ref(), &codex_resume)?
                } else {
                    native
                };
                if capture.workspace_id != workspace_id
                    || capture.tab_id != tab_id
                    || capture.agent_session != occupant
                    || capture.agent_name != saved.agent_name
                {
                    return Err(mismatch("migration frozen occupant differs from capture"));
                }
                let terminal_id = TerminalId::checked_import(capture.terminal_id.clone())?;
                let admitted = match (&capture.child, &capture.unavailable) {
                    (Some(child), None) => {
                        let runtime = runtime_by_raw
                            .get(&raw)
                            .ok_or_else(|| mismatch("migration frozen live pane has no runtime"))?;
                        if child.pid != runtime.child_pid {
                            return Err(mismatch("migration frozen child differs from capture"));
                        }
                        AdmittedPaneIdentity::Runtime(terminal_id)
                    }
                    (None, Some(state)) => {
                        if runtime_by_raw.contains_key(&raw)
                            || saved.cwd != state.cwd
                            || saved.label != state.label
                            || saved.managed_agent_kind.is_some()
                            || saved.agent_resume.is_some()
                        {
                            return Err(mismatch(
                                "migration frozen unavailable placeholder differs",
                            ));
                        }
                        AdmittedPaneIdentity::Unavailable(
                            crate::handoff_runtime::ImportedHandoffPlaceholder {
                                terminal_id,
                                observation_epoch: receipt.observation_epoch.clone(),
                                state: state.clone(),
                            },
                        )
                    }
                    _ => return Err(mismatch("migration admitted runtime presence differs")),
                };
                identities.insert(raw, admitted);
            }
        }
        if workspace_raw_ids.len() != workspace.public_pane_numbers.len() {
            return Err(mismatch("migration public pane table has extra identities"));
        }
    }
    let live_count = identities
        .values()
        .filter(|identity| matches!(identity, AdmittedPaneIdentity::Runtime(_)))
        .count();
    if identities.len() != captured.len() || live_count != runtime_by_raw.len() {
        return Err(mismatch(
            "migration capture/snapshot/runtime coverage differs",
        ));
    }
    Ok(identities)
}

/// serde_json ordinarily overwrites duplicate object keys. A preservation
/// receipt/manifest must retain one unambiguous identity at every nesting level.
pub(crate) fn parse_unique_json(bytes: &[u8]) -> io::Result<Value> {
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("JSON without duplicate keys")
                }
                fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Unique, E> {
                    Ok(Unique(Value::Bool(value)))
                }
                fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Unique, E> {
                    Ok(Unique(value.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Unique, E> {
                    Ok(Unique(value.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Unique, E> {
                    serde_json::Number::from_f64(value)
                        .map(|n| Unique(Value::Number(n)))
                        .ok_or_else(|| E::custom("nonfinite JSON number"))
                }
                fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Unique, E> {
                    Ok(Unique(Value::String(value.to_string())))
                }
                fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Unique, E> {
                    Ok(Unique(Value::String(value)))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_none<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut sequence: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(value)) = sequence.next_element()? {
                        values.push(value);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                        if values.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate JSON object key"));
                        }
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Unique>(bytes)
        .map(|unique| unique.0)
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixtures() -> (IdentityReceipt, HandoffManifest) {
        let process = serde_json::json!({"pid": 10,"uid": 1000,"start_ticks": 123,"boot_id":"boot","pid_namespace":"pid:[1]","mount_namespace":"mnt:[1]"});
        let file = serde_json::json!({"device":1,"inode":2,"uid":1000});
        let mut child = process.clone();
        child["pid"] = 42.into();
        let receipt = serde_json::from_value(serde_json::json!({
            "version":1,"operation_id":"fresh-epoch","observation_epoch":"fresh-epoch", "source":process,
            "source_sha256":"old","source_version":"0.9.3","source_protocol":16,"candidate_sha256":"new","candidate_version":"0.9.3","candidate_protocol":16,
            "session":null,"config_path":"/config/config.toml","data_dir":"/config","data_dir_stamp":file,"api_socket":"/config/herdr.sock","api_socket_stamp":file,"client_socket":"/config/herdr-client.sock","client_socket_stamp":file,
            "captured_agents":[],"panes":[{"pane_id":"wA:p1G","terminal_id":"opaque-exact-old-ID","workspace_id":"wA","tab_id":"wA:tD","agent_name":null,"agent_session":null,"child":child}]
        })).unwrap();
        let manifest = serde_json::from_value(serde_json::json!({
            "version":1,"source_version":"0.9.3","source_protocol":16,"expected_version":"0.9.3","expected_protocol":16,
            "snapshot":{"version":3,"active":0,"selected":0,"workspaces":[{"id":"wA","identity_cwd":"/missing-is-valid-for-import","public_pane_numbers":{"900":48},"public_tab_numbers":[13],"tabs":[{"layout":{"Pane":900},"panes":{"900":{"cwd":"/missing-is-valid-for-import"}},"zoomed":false,"focused":900,"root_pane":900}]}]},
            "panes":[{"pane_id":900,"child_pid":42,"rows":24,"cols":80,"cell_width_px":0,"cell_height_px":0}]
        })).unwrap();
        (receipt, manifest)
    }
    #[test]
    fn sparse_canonical_mapping_preserves_exact_terminal_id() {
        let (receipt, manifest) = fixtures();
        let admitted = admit_manifest(&receipt, &manifest, |_| None).unwrap();
        let AdmittedPaneIdentity::Runtime(id) = &admitted[&900] else {
            panic!("live pane was admitted as unavailable");
        };
        assert_eq!(id.as_str(), "opaque-exact-old-ID");
    }
    #[test]
    fn missing_runtime_duplicate_and_changed_occupant_are_not_admitted() {
        let (mut receipt, mut manifest) = fixtures();
        manifest.panes.clear();
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        let (_, valid) = fixtures();
        manifest.panes = valid.panes;
        manifest.panes.push(manifest.panes[0].clone());
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        manifest.panes.pop();
        receipt.panes[0].child.as_mut().unwrap().pid += 1;
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        receipt.panes[0].child.as_mut().unwrap().pid -= 1;
        receipt.panes[0].terminal_id.clear();
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
    }
    #[test]
    fn incomplete_public_numbers_and_layout_are_not_synthesized() {
        let (receipt, mut manifest) = fixtures();
        manifest.snapshot.workspaces[0].public_pane_numbers.clear();
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        let (_, mut manifest) = fixtures();
        manifest.snapshot.workspaces[0].tabs[0].root_pane = Some(901);
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        let (_, mut manifest) = fixtures();
        manifest.snapshot.workspaces[0].tabs[0].panes.clear();
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
    }
    #[test]
    fn duplicate_nested_keys_are_rejected_without_losing_u64_precision() {
        assert!(parse_unique_json(br#"{"a":{"pane_id":1,"pane_id":2}}"#).is_err());
        let value = parse_unique_json(br#"{"sequence":18446744073709551615}"#).unwrap();
        assert_eq!(value["sequence"].as_u64(), Some(u64::MAX));
    }

    /// A pane whose saved conversation carries launch options and an account,
    /// captured from the source API, which publishes only the conversation
    /// identity.
    fn fixtures_with_agent_session() -> (IdentityReceipt, HandoffManifest) {
        let (mut receipt, mut manifest) = fixtures();
        receipt.panes[0].agent_name = Some("worker".into());
        receipt.panes[0].agent_session = Some(serde_json::json!({
            "agent":"claude","kind":"id","source":"herdr:claude","value":"conversation-1"
        }));
        let saved = manifest.snapshot.workspaces[0].tabs[0]
            .panes
            .get_mut(&900)
            .unwrap();
        saved.agent_name = Some("worker".into());
        saved.agent_session = Some(serde_json::from_value(serde_json::json!({
            "source":"herdr:claude","agent":"claude","kind":"id","value":"conversation-1",
            "launch_options":["--model","claude-haiku-5-5","--permission-mode","bypassPermissions"],
            "launch_account":"claude:a@example.com","resumed_without_launch_options":true
        })).unwrap());
        (receipt, manifest)
    }

    #[test]
    fn saved_launch_options_and_account_do_not_change_the_occupant_identity() {
        let (receipt, manifest) = fixtures_with_agent_session();
        let admitted = admit_manifest(&receipt, &manifest, |_| None).unwrap();
        assert!(
            matches!(&admitted[&900], AdmittedPaneIdentity::Runtime(id) if id.as_str() == "opaque-exact-old-ID")
        );
        // The saved options and account stay in the manifest the import restores from.
        let saved = manifest.snapshot.workspaces[0].tabs[0].panes[&900]
            .agent_session
            .as_ref()
            .unwrap();
        assert_eq!(
            saved.launch_account.as_deref(),
            Some("claude:a@example.com")
        );
        assert_eq!(saved.launch_options.as_ref().map(Vec::len), Some(4));
    }

    #[test]
    fn a_different_captured_conversation_is_still_not_admitted() {
        for (field, value) in [
            ("value", "conversation-2"),
            ("source", "herdr:codex"),
            ("agent", "codex"),
            ("kind", "path"),
        ] {
            let (mut receipt, manifest) = fixtures_with_agent_session();
            receipt.panes[0].agent_session.as_mut().unwrap()[field] = value.into();
            let error = admit_manifest(&receipt, &manifest, |_| None).err().unwrap();
            assert_eq!(
                error.to_string(),
                "migration frozen occupant differs from capture",
                "{field}"
            );
        }
        let (mut receipt, manifest) = fixtures_with_agent_session();
        receipt.panes[0].agent_session = None;
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        let (mut receipt, manifest) = fixtures_with_agent_session();
        receipt.panes[0].agent_name = None;
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        let (receipt, mut manifest) = fixtures_with_agent_session();
        manifest.snapshot.workspaces[0].tabs[0]
            .panes
            .get_mut(&900)
            .unwrap()
            .agent_session = None;
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
    }

    #[test]
    fn explicit_unavailable_placeholder_requires_exact_snapshot_and_no_runtime() {
        let (mut receipt, mut manifest) = fixtures();
        let saved = &mut manifest.snapshot.workspaces[0].tabs[0].panes;
        saved.get_mut(&900).unwrap().label = Some("saved label".into());
        receipt.panes[0].child = None;
        receipt.panes[0].unavailable = Some(crate::handoff_runtime::UnavailableHandoffState {
            restore_error: crate::handoff_runtime::SAVED_DIRECTORY_UNAVAILABLE.into(),
            cwd: "/missing-is-valid-for-import".into(),
            label: Some("saved label".into()),
        });
        let exported_runtime = manifest.panes.pop().unwrap();
        let admitted = admit_manifest(&receipt, &manifest, |_| None).unwrap();
        let AdmittedPaneIdentity::Unavailable(placeholder) = &admitted[&900] else {
            panic!("unavailable pane was admitted as live");
        };
        assert_eq!(placeholder.terminal_id.as_str(), "opaque-exact-old-ID");
        assert_eq!(placeholder.observation_epoch, "fresh-epoch");
        assert_eq!(
            placeholder.state,
            receipt.panes[0].unavailable.clone().unwrap()
        );
        manifest.panes.push(exported_runtime);
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        manifest.panes.clear();
        receipt.panes[0].unavailable.as_mut().unwrap().cwd = "/changed".into();
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
        receipt.panes[0].unavailable.as_mut().unwrap().cwd = "/missing-is-valid-for-import".into();
        receipt.panes[0].unavailable.as_mut().unwrap().restore_error = "different failure".into();
        assert!(admit_manifest(&receipt, &manifest, |_| None).is_err());
    }

    /// A Codex resumed by a command typed into the pane's shell: the source
    /// API names its conversation from that command line, and the frozen
    /// snapshot saves none.
    fn fixtures_with_unsaved_codex_resume() -> (IdentityReceipt, HandoffManifest) {
        let (mut receipt, manifest) = fixtures();
        receipt.panes[0].agent_session = Some(serde_json::json!({
            "agent":"codex","kind":"id","source":"herdr:codex","value":"conversation-1"
        }));
        (receipt, manifest)
    }

    #[test]
    fn an_unsaved_codex_resume_is_admitted_while_the_pane_codex_resumes_it() {
        let (receipt, manifest) = fixtures_with_unsaved_codex_resume();
        let admitted = admit_manifest(&receipt, &manifest, |shell| {
            (shell == 42).then(|| "conversation-1".to_string())
        })
        .unwrap();
        assert!(
            matches!(&admitted[&900], AdmittedPaneIdentity::Runtime(id) if id.as_str() == "opaque-exact-old-ID")
        );
    }

    #[test]
    fn an_unsaved_conversation_the_pane_codex_does_not_resume_is_not_admitted() {
        for resumed in [None, Some("conversation-2")] {
            let (receipt, manifest) = fixtures_with_unsaved_codex_resume();
            let error = admit_manifest(&receipt, &manifest, |_| resumed.map(String::from))
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                "migration frozen occupant differs from capture",
                "{resumed:?}"
            );
        }
        let resumes_it = |_| Some("conversation-1".to_string());
        // Only a Codex conversation id is read from a Codex command line.
        for (field, value) in [
            ("source", "herdr:claude"),
            ("agent", "claude"),
            ("kind", "path"),
        ] {
            let (mut receipt, manifest) = fixtures_with_unsaved_codex_resume();
            receipt.panes[0].agent_session.as_mut().unwrap()[field] = value.into();
            assert!(
                admit_manifest(&receipt, &manifest, resumes_it).is_err(),
                "{field}"
            );
        }
        // Another agent name or child process still differs.
        let (mut receipt, manifest) = fixtures_with_unsaved_codex_resume();
        receipt.panes[0].agent_name = Some("worker".into());
        assert!(admit_manifest(&receipt, &manifest, resumes_it).is_err());
        let (mut receipt, manifest) = fixtures_with_unsaved_codex_resume();
        receipt.panes[0].child.as_mut().unwrap().pid += 1;
        assert!(admit_manifest(&receipt, &manifest, resumes_it).is_err());
    }

    #[test]
    fn a_saved_conversation_is_never_replaced_by_the_command_line() {
        let (mut receipt, manifest) = fixtures_with_agent_session();
        receipt.panes[0].agent_session = Some(serde_json::json!({
            "agent":"codex","kind":"id","source":"herdr:codex","value":"conversation-2"
        }));
        let read = std::cell::Cell::new(false);
        let error = admit_manifest(&receipt, &manifest, |_| {
            read.set(true);
            Some("conversation-2".to_string())
        })
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "migration frozen occupant differs from capture"
        );
        assert!(!read.get());
        let (receipt, manifest) = fixtures_with_agent_session();
        admit_manifest(&receipt, &manifest, |_| {
            read.set(true);
            None
        })
        .unwrap();
        assert!(!read.get());
    }
}
