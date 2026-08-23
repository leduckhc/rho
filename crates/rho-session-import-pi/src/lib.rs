//! One-way import of a pi session file into rho's record set.
//!
//! Feature F-pi-session-import. See `SPEC-sessions` section 9. The conversion is one-way: it reads a pi
//! JSONL file and returns rho `Entry` records. It never changes the pi file.

use std::collections::BTreeMap;
use std::path::Path;

use rho_core::{ContentBlock, Entry, ImageSource, Message, Record, RecordId, Role, SessionError};
use serde_json::Value;

/// rho writes its own session version, not pi's `3`. See SPEC-sessions section 9.
const RHO_SESSION_VERSION: u32 = 1;

/// The strictest resolved modes. A pi header names neither, so an import assumes the
/// safest possible modes rather than the loosest. See SPEC-sessions section 8a.
const DEFAULT_APPROVAL: &str = "read-only";
const DEFAULT_SANDBOX: &str = "strict";

/// The pi record types rho has no model for, and so drops. See SPEC-sessions section 9.
///
/// A real pi file holds every one of these. `custom_message` and `compaction` are here
/// because a run over 60 real files failed on them. See decision D-unmappable-pi-record-drops.
const KNOWN_DROPPABLE: &[&str] = &[
    "thinking_level_change",
    "session_info",
    "custom",
    "custom_message",
    "compaction",
];

/// What one import produced, and what it dropped.
///
/// The dropped counts make a loss visible. An import that returned only the entries
/// would hide a whole record type behind a shorter list. See SPEC-sessions section 9.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PiImport {
    /// The records that mapped, in file order.
    pub entries: Vec<Entry>,
    /// How many records dropped, by the pi type or by the reason.
    pub dropped: BTreeMap<String, usize>,
}

/// Convert a pi session file to rho records.
///
/// It reads the JSONL file at `pi_path` and maps each pi record to a rho `Record`,
/// per the mapping in `SPEC-sessions` section 9. It keeps every `id`, every `parentId`, and
/// every `timestamp`, so the tree shape survives. It never changes the pi file.
///
/// A record rho cannot map drops, and the drop is counted by the pi type or by the
/// reason. The import does not stop on a record it does not know, because a real pi file
/// holds shapes that no fixture predicted. See decision D-unmappable-pi-record-drops.
pub fn import_pi_session(pi_path: &Path) -> Result<PiImport, SessionError> {
    let contents = std::fs::read_to_string(pi_path).map_err(|e| SessionError::Io(e.to_string()))?;

    let mut import = PiImport::default();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).map_err(|e| SessionError::Decode(e.to_string()))?;
        match map_record(&value) {
            Ok(Some(entry)) => import.entries.push(entry),
            Ok(None) => count(&mut import.dropped, pi_type_of(&value)),
            Err(Drop { reason }) => count(&mut import.dropped, reason),
        }
    }
    Ok(import)
}

/// A record rho cannot map. The reason names the pi type or the pi role, so a caller can
/// see what it lost.
struct Drop {
    reason: String,
}

/// The pi `type` of a record, or a stand-in when the record has none.
fn pi_type_of(value: &Value) -> String {
    value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("a record with no type")
        .to_string()
}

/// Add one to the count for `reason`.
fn count(dropped: &mut BTreeMap<String, usize>, reason: String) {
    *dropped.entry(reason).or_insert(0) += 1;
}

/// Map one pi record to a rho `Entry`, or `None` when the type is a known droppable.
///
/// An `Err(Drop)` means the record cannot map and must be counted, never that the file is
/// unreadable. A malformed line is caught by the caller, before this function.
fn map_record(value: &Value) -> Result<Option<Entry>, Drop> {
    let Some(pi_type) = value.get("type").and_then(Value::as_str) else {
        return Err(Drop {
            reason: "a record with no type".to_string(),
        });
    };

    if KNOWN_DROPPABLE.contains(&pi_type) {
        return Ok(None);
    }

    let record = match pi_type {
        "session" => Record::Session {
            version: RHO_SESSION_VERSION,
            cwd: string_field(value, "cwd").unwrap_or_default().into(),
            approval: DEFAULT_APPROVAL.to_string(),
            sandbox: DEFAULT_SANDBOX.to_string(),
        },
        "model_change" => {
            // The fixture names the field `model`; a real pi file names it `modelId`.
            // So the importer accepts both spellings. See SPEC-sessions section 9.
            let provider = string_field(value, "provider").ok_or_else(|| Drop {
                reason: "model_change with no provider".to_string(),
            })?;
            let model = string_field(value, "model")
                .or_else(|| string_field(value, "modelId"))
                .ok_or_else(|| Drop {
                    reason: "model_change with no model".to_string(),
                })?;
            Record::ModelChange { provider, model }
        }
        "message" => {
            let body = value.get("message").ok_or_else(|| Drop {
                reason: "message with no body".to_string(),
            })?;
            Record::Message {
                message: map_message(body)?,
            }
        }
        other => {
            // A type this build does not know drops, and the count names it. An error here
            // would stop a whole import for one auxiliary record, and a run over 60 real
            // pi files proved that 21 of them hold such a record. See D-unmappable-pi-record-drops.
            return Err(Drop {
                reason: other.to_string(),
            });
        }
    };

    let id = string_field(value, "id").ok_or_else(|| Drop {
        reason: format!("{pi_type} with no id"),
    })?;
    let parent_id = string_field(value, "parentId").map(RecordId);
    let timestamp = string_field(value, "timestamp").unwrap_or_default();

    Ok(Some(Entry {
        id: RecordId(id),
        parent_id,
        timestamp,
        record,
    }))
}

/// Map a pi message body to a rho `Message`.
///
/// A role rho has no model for drops, and the count names the role. `bashExecution` is
/// one such role in a real pi file today.
fn map_message(body: &Value) -> Result<Message, Drop> {
    let role = match body.get("role").and_then(Value::as_str) {
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        Some("toolResult") => Role::Tool,
        Some(other) => {
            return Err(Drop {
                reason: format!("message with the role {other}"),
            });
        }
        None => {
            return Err(Drop {
                reason: "message with no role".to_string(),
            });
        }
    };

    let content = map_content(body.get("content"))?;
    Ok(Message { role, content })
}

/// Map a pi content array to rho `ContentBlock`s.
fn map_content(content: Option<&Value>) -> Result<Vec<ContentBlock>, Drop> {
    let Some(array) = content.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    array.iter().map(map_content_block).collect()
}

/// Map one pi content block to a rho `ContentBlock`.
fn map_content_block(block: &Value) -> Result<ContentBlock, Drop> {
    let block_type = block
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| Drop {
            reason: "content block with no type".to_string(),
        })?;

    match block_type {
        "text" => Ok(ContentBlock::Text {
            text: string_field(block, "text").unwrap_or_default(),
        }),
        // A trace, never a replay block. pi's `thinkingSignature` belongs to a provider
        // and a model that this file does not record, so rho can build no honest owner.
        // A guessed owner would be replayed, and rule 8 exists to refuse exactly that.
        // The text is kept, and the signature is dropped.
        "thinking" => Ok(ContentBlock::ReasoningTrace {
            text: string_field(block, "thinking").unwrap_or_default(),
        }),
        "toolCall" => Ok(ContentBlock::ToolCall {
            id: string_field(block, "id").ok_or_else(|| Drop {
                reason: "toolCall with no id".to_string(),
            })?,
            name: string_field(block, "name").ok_or_else(|| Drop {
                reason: "toolCall with no name".to_string(),
            })?,
            arguments: block.get("arguments").cloned().unwrap_or(Value::Null),
            // pi records no replay payload for a call, and a guessed one would travel.
            state: None,
        }),
        "toolResult" => Ok(ContentBlock::ToolResult {
            tool_call_id: string_field(block, "tool_call_id").ok_or_else(|| Drop {
                reason: "toolResult with no tool_call_id".to_string(),
            })?,
            content: map_content(block.get("content"))?,
            is_error: block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        // A real pi file holds 170 image blocks. A pi block names the fields `data` and
        // `mimeType`. See SPEC-sessions section 9.
        "image" => Ok(ContentBlock::Image {
            source: ImageSource {
                data: string_field(block, "data").ok_or_else(|| Drop {
                    reason: "image with no data".to_string(),
                })?,
                mime_type: string_field(block, "mimeType")
                    .or_else(|| string_field(block, "mime_type"))
                    .ok_or_else(|| Drop {
                        reason: "image with no mimeType".to_string(),
                    })?,
            },
        }),
        other => Err(Drop {
            reason: format!("content block of type {other}"),
        }),
    }
}

/// Read a string field from a JSON object, if present and a string.
fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}
