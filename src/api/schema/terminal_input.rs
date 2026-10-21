//! Acknowledged, de-duplicated terminal input (`terminal.input.*`).
//!
//! Advertised by `ServerCapabilities::terminal_input_ack`. These methods never
//! change the meaning of `pane.send_input` or `pane.send_text`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalInputOpenParams {
    pub terminal_id: String,
    /// Incarnation the client is bound to. Omit to bind to the current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalInputWriteParams {
    pub terminal_id: String,
    pub incarnation: String,
    pub stream_id: String,
    /// Starts at 1 and increases by exactly 1 per write on the stream.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalInputReconcileParams {
    pub terminal_id: String,
    pub incarnation: String,
    pub stream_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalInputStreamInfo {
    pub terminal_id: String,
    pub incarnation: String,
    pub stream_id: String,
    /// Highest sequence admitted to the terminal's ordered PTY write queue.
    pub last_admitted_seq: u64,
    /// Every sequence up to this one was written to the PTY.
    pub last_written_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalInputWriteAck {
    pub terminal_id: String,
    pub incarnation: String,
    pub stream_id: String,
    pub seq: u64,
    /// Hex SHA-256 of the request's text and keys.
    pub digest: String,
    /// True when an earlier request with this sequence already wrote it.
    pub duplicate: bool,
}
