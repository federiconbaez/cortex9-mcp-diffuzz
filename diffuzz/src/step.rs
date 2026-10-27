//! The message vocabulary of the fuzzer.
//!
//! A [`Step`] is one line written to the server's stdin. Steps are kept
//! symbolic (not raw bytes) so that cases stay small on disk, can be
//! minimized structurally and can be rendered again byte for byte.

use serde_json::{json, Value};

/// Protocol revisions offered in `initialize`. `1999-01-01` is deliberately
/// unsupported.
pub const VERSIONS: &[&str] = &[
    "2025-03-26",
    "2025-06-18",
    "2025-11-25",
    "2026-07-28",
    "1999-01-01",
];

/// Revision used for the stateless (`_meta`) requests of `2026-07-28`.
pub const MODERN: &str = "2026-07-28";

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Initialize {
        id: i64,
        version: String,
    },
    InitializeNoParams {
        id: i64,
    },
    /// `initialize` sent as a notification (no id).
    InitializeAsNotification,
    Initialized,
    ToolsList {
        id: i64,
    },
    /// `tools/list` with the stateless `_meta` of `2026-07-28`.
    ModernToolsList {
        id: i64,
    },
    CallEcho {
        id: i64,
        text: String,
    },
    /// 0: missing `text`, 1: `text` is a number, 2: `arguments: null`,
    /// 3: no `arguments`.
    CallEchoBadArgs {
        id: i64,
        variant: u8,
    },
    CallUnknownTool {
        id: i64,
    },
    Ping {
        id: i64,
    },
    PingStrId {
        id: String,
    },
    UnknownMethod {
        id: i64,
    },
    ParamsArray {
        id: i64,
    },
    NullId,
    FloatId,
    /// id = u64::MAX, outside the i64 range.
    HugeId,
    MissingJsonrpc {
        id: i64,
    },
    WrongJsonrpc {
        id: i64,
    },
    MethodNotString {
        id: i64,
    },
    /// 0: `null`, 1: `"x"`, 2: `42`, 3: `{}`.
    NonObject {
        variant: u8,
    },
    /// 0: truncated object, 1: garbage, 2: request with a lone UTF-16
    /// surrogate escape, 3: request with invalid UTF-8 bytes.
    Malformed {
        variant: u8,
    },
    EmptyBatch,
    Batch {
        items: Vec<Step>,
    },
    Cancelled {
        request_id: i64,
    },
    ClientResponse {
        id: i64,
    },
    UnknownNotification,
    /// A `ping` padded with `bytes` extra bytes on one line.
    Oversized {
        id: i64,
        bytes: usize,
    },
}

/// What a well-behaved server must send back for a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Expect {
    /// Nothing: the step is a notification or a client response.
    Nothing,
    /// One response (possibly an error with `id: null`).
    Reply,
}

fn req(id: Value, method: &str, params: Option<Value>) -> Value {
    let mut v = json!({ "jsonrpc": "2.0", "id": id, "method": method });
    if let Some(p) = params {
        v["params"] = p;
    }
    v
}

fn notif(method: &str, params: Option<Value>) -> Value {
    let mut v = json!({ "jsonrpc": "2.0", "method": method });
    if let Some(p) = params {
        v["params"] = p;
    }
    v
}

fn init_params(version: &str) -> Value {
    json!({
        "protocolVersion": version,
        "capabilities": {},
        "clientInfo": { "name": "mcp-diffuzz", "version": "0.1.0" }
    })
}

fn modern_meta() -> Value {
    json!({ "_meta": {
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {}
    }})
}

impl Step {
    /// Short stable name, used in signatures.
    pub fn op(&self) -> String {
        match self {
            Step::Initialize { .. } => "initialize".into(),
            Step::InitializeNoParams { .. } => "initialize-no-params".into(),
            Step::InitializeAsNotification => "initialize-as-notification".into(),
            Step::Initialized => "initialized".into(),
            Step::ToolsList { .. } => "tools-list".into(),
            Step::ModernToolsList { .. } => "modern-tools-list".into(),
            Step::CallEcho { .. } => "call-echo".into(),
            Step::CallEchoBadArgs { variant, .. } => format!("call-echo-bad-args-{variant}"),
            Step::CallUnknownTool { .. } => "call-unknown-tool".into(),
            Step::Ping { .. } => "ping".into(),
            Step::PingStrId { .. } => "ping-str-id".into(),
            Step::UnknownMethod { .. } => "unknown-method".into(),
            Step::ParamsArray { .. } => "params-array".into(),
            Step::NullId => "null-id".into(),
            Step::FloatId => "float-id".into(),
            Step::HugeId => "huge-id".into(),
            Step::MissingJsonrpc { .. } => "missing-jsonrpc".into(),
            Step::WrongJsonrpc { .. } => "wrong-jsonrpc".into(),
            Step::MethodNotString { .. } => "method-not-string".into(),
            Step::NonObject { variant } => format!("non-object-{variant}"),
            Step::Malformed { variant } => format!("malformed-{variant}"),
            Step::EmptyBatch => "empty-batch".into(),
            // Item lists are not part of the name: batches that differ only
            // in their items share a signature.
            Step::Batch { items } => {
                if items.iter().all(|i| i.expect() == Expect::Nothing) {
                    "batch-of-notifications".into()
                } else {
                    "batch".into()
                }
            }
            Step::Cancelled { .. } => "cancelled".into(),
            Step::ClientResponse { .. } => "client-response".into(),
            Step::UnknownNotification => "unknown-notification".into(),
            Step::Oversized { .. } => "oversized".into(),
        }
    }

    /// The JSON value of the step, if it is valid JSON.
    pub fn value(&self) -> Option<Value> {
        Some(match self {
            Step::Initialize { id, version } => {
                req(json!(id), "initialize", Some(init_params(version)))
            }
            Step::InitializeNoParams { id } => req(json!(id), "initialize", None),
            Step::InitializeAsNotification => notif("initialize", Some(init_params("2025-11-25"))),
            Step::Initialized => notif("notifications/initialized", None),
            Step::ToolsList { id } => req(json!(id), "tools/list", None),
            Step::ModernToolsList { id } => req(json!(id), "tools/list", Some(modern_meta())),
            Step::CallEcho { id, text } => req(
                json!(id),
                "tools/call",
                Some(json!({ "name": "echo", "arguments": { "text": text } })),
            ),
            Step::CallEchoBadArgs { id, variant } => {
                let params = match variant {
                    0 => json!({ "name": "echo", "arguments": {} }),
                    1 => json!({ "name": "echo", "arguments": { "text": 1 } }),
                    2 => json!({ "name": "echo", "arguments": null }),
                    _ => json!({ "name": "echo" }),
                };
                req(json!(id), "tools/call", Some(params))
            }
            Step::CallUnknownTool { id } => req(
                json!(id),
                "tools/call",
                Some(json!({ "name": "no_such_tool", "arguments": {} })),
            ),
            Step::Ping { id } => req(json!(id), "ping", None),
            Step::PingStrId { id } => req(json!(id), "ping", None),
            Step::UnknownMethod { id } => req(json!(id), "diffuzz/unknown", None),
            Step::ParamsArray { id } => req(json!(id), "tools/list", Some(json!([]))),
            Step::NullId => req(Value::Null, "ping", None),
            Step::FloatId => req(json!(1.5), "ping", None),
            Step::HugeId => req(json!(u64::MAX), "ping", None),
            Step::MissingJsonrpc { id } => json!({ "id": id, "method": "ping" }),
            Step::WrongJsonrpc { id } => json!({ "jsonrpc": "1.0", "id": id, "method": "ping" }),
            Step::MethodNotString { id } => json!({ "jsonrpc": "2.0", "id": id, "method": 5 }),
            Step::NonObject { variant } => match variant {
                0 => Value::Null,
                1 => json!("x"),
                2 => json!(42),
                _ => json!({}),
            },
            Step::EmptyBatch => json!([]),
            Step::Batch { items } => Value::Array(items.iter().filter_map(Step::value).collect()),
            Step::Cancelled { request_id } => notif(
                "notifications/cancelled",
                Some(json!({ "requestId": request_id, "reason": "diffuzz" })),
            ),
            Step::ClientResponse { id } => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            Step::UnknownNotification => notif("notifications/diffuzz", None),
            Step::Malformed { .. } | Step::Oversized { .. } => return None,
        })
    }

    /// The id a reply should carry, when the step is a request with one.
    pub fn request_id(&self) -> Option<Value> {
        match self {
            Step::Oversized { id, .. } => Some(json!(id)),
            other => other.value()?.get("id").cloned(),
        }
    }

    /// Bytes of the line, without the trailing newline. Never contains `\n`.
    pub fn render(&self) -> Vec<u8> {
        match self {
            Step::Malformed { variant } => match variant {
                0 => br#"{"jsonrpc":"2.0","id":1,"method":"pi"#.to_vec(),
                1 => b"diffuzz \x01 not json".to_vec(),
                2 => br#"{"jsonrpc":"2.0","id":77,"method":"tools/call","params":{"name":"echo","arguments":{"text":"a\ud800b"}}}"#.to_vec(),
                _ => {
                    let mut v = br#"{"jsonrpc":"2.0","id":78,"method":"ping","params":{"x":""#.to_vec();
                    v.extend_from_slice(b"\xff\xfe");
                    v.extend_from_slice(br#""}}"#);
                    v
                }
            },
            Step::Oversized { id, bytes } => {
                let mut v = format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"method":"ping","params":{{"pad":""#
                )
                .into_bytes();
                v.resize(v.len() + bytes, b'a');
                v.extend_from_slice(br#""}}"#);
                v
            }
            other => serde_json::to_vec(&other.value().expect("valid JSON step"))
                .expect("serializable"),
        }
    }

    pub fn expect(&self) -> Expect {
        match self {
            Step::InitializeAsNotification
            | Step::Initialized
            | Step::Cancelled { .. }
            | Step::ClientResponse { .. }
            | Step::UnknownNotification => Expect::Nothing,
            Step::Batch { items } => {
                if items.iter().all(|i| i.expect() == Expect::Nothing) {
                    Expect::Nothing
                } else {
                    Expect::Reply
                }
            }
            _ => Expect::Reply,
        }
    }

    /// Whether this is a well-formed notification/response that must get no
    /// reply at all (JSON-RPC 2.0, section 4.1).
    pub fn is_valid_silent(&self) -> bool {
        self.expect() == Expect::Nothing
    }

    pub fn to_json(&self) -> Value {
        match self {
            Step::Initialize { id, version } => {
                json!({ "op": "initialize", "id": id, "version": version })
            }
            Step::InitializeNoParams { id } => json!({ "op": "initialize-no-params", "id": id }),
            Step::InitializeAsNotification => json!({ "op": "initialize-as-notification" }),
            Step::Initialized => json!({ "op": "initialized" }),
            Step::ToolsList { id } => json!({ "op": "tools-list", "id": id }),
            Step::ModernToolsList { id } => json!({ "op": "modern-tools-list", "id": id }),
            Step::CallEcho { id, text } => json!({ "op": "call-echo", "id": id, "text": text }),
            Step::CallEchoBadArgs { id, variant } => {
                json!({ "op": "call-echo-bad-args", "id": id, "variant": variant })
            }
            Step::CallUnknownTool { id } => json!({ "op": "call-unknown-tool", "id": id }),
            Step::Ping { id } => json!({ "op": "ping", "id": id }),
            Step::PingStrId { id } => json!({ "op": "ping-str-id", "id": id }),
            Step::UnknownMethod { id } => json!({ "op": "unknown-method", "id": id }),
            Step::ParamsArray { id } => json!({ "op": "params-array", "id": id }),
            Step::NullId => json!({ "op": "null-id" }),
            Step::FloatId => json!({ "op": "float-id" }),
            Step::HugeId => json!({ "op": "huge-id" }),
            Step::MissingJsonrpc { id } => json!({ "op": "missing-jsonrpc", "id": id }),
            Step::WrongJsonrpc { id } => json!({ "op": "wrong-jsonrpc", "id": id }),
            Step::MethodNotString { id } => json!({ "op": "method-not-string", "id": id }),
            Step::NonObject { variant } => json!({ "op": "non-object", "variant": variant }),
            Step::Malformed { variant } => json!({ "op": "malformed", "variant": variant }),
            Step::EmptyBatch => json!({ "op": "empty-batch" }),
            Step::Batch { items } => {
                json!({ "op": "batch", "items": items.iter().map(Step::to_json).collect::<Vec<_>>() })
            }
            Step::Cancelled { request_id } => {
                json!({ "op": "cancelled", "request_id": request_id })
            }
            Step::ClientResponse { id } => json!({ "op": "client-response", "id": id }),
            Step::UnknownNotification => json!({ "op": "unknown-notification" }),
            Step::Oversized { id, bytes } => json!({ "op": "oversized", "id": id, "bytes": bytes }),
        }
    }

    pub fn from_json(v: &Value) -> Result<Step, String> {
        let op = v["op"].as_str().ok_or("step without op")?;
        let id = || {
            v["id"]
                .as_i64()
                .ok_or(format!("{op}: id must be an integer"))
        };
        let variant = || {
            v["variant"]
                .as_u64()
                .map(|n| n as u8)
                .ok_or(format!("{op}: missing variant"))
        };
        Ok(match op {
            "initialize" => Step::Initialize {
                id: id()?,
                version: v["version"].as_str().ok_or("initialize: version")?.into(),
            },
            "initialize-no-params" => Step::InitializeNoParams { id: id()? },
            "initialize-as-notification" => Step::InitializeAsNotification,
            "initialized" => Step::Initialized,
            "tools-list" => Step::ToolsList { id: id()? },
            "modern-tools-list" => Step::ModernToolsList { id: id()? },
            "call-echo" => Step::CallEcho {
                id: id()?,
                text: v["text"].as_str().ok_or("call-echo: text")?.into(),
            },
            "call-echo-bad-args" => Step::CallEchoBadArgs {
                id: id()?,
                variant: variant()?,
            },
            "call-unknown-tool" => Step::CallUnknownTool { id: id()? },
            "ping" => Step::Ping { id: id()? },
            "ping-str-id" => Step::PingStrId {
                id: v["id"].as_str().ok_or("ping-str-id: id")?.into(),
            },
            "unknown-method" => Step::UnknownMethod { id: id()? },
            "params-array" => Step::ParamsArray { id: id()? },
            "null-id" => Step::NullId,
            "float-id" => Step::FloatId,
            "huge-id" => Step::HugeId,
            "missing-jsonrpc" => Step::MissingJsonrpc { id: id()? },
            "wrong-jsonrpc" => Step::WrongJsonrpc { id: id()? },
            "method-not-string" => Step::MethodNotString { id: id()? },
            "non-object" => Step::NonObject {
                variant: variant()?,
            },
            "malformed" => Step::Malformed {
                variant: variant()?,
            },
            "empty-batch" => Step::EmptyBatch,
            "batch" => Step::Batch {
                items: v["items"]
                    .as_array()
                    .ok_or("batch: items")?
                    .iter()
                    .map(Step::from_json)
                    .collect::<Result<_, _>>()?,
            },
            "cancelled" => Step::Cancelled {
                request_id: v["request_id"].as_i64().ok_or("cancelled: request_id")?,
            },
            "client-response" => Step::ClientResponse { id: id()? },
            "unknown-notification" => Step::UnknownNotification,
            "oversized" => Step::Oversized {
                id: id()?,
                bytes: v["bytes"].as_u64().ok_or("oversized: bytes")? as usize,
            },
            other => return Err(format!("unknown op '{other}'")),
        })
    }
}

#[cfg(test)]
pub(crate) fn all_examples() -> Vec<Step> {
    let mut v = vec![
        Step::Initialize {
            id: 1,
            version: "2025-11-25".into(),
        },
        Step::InitializeNoParams { id: 2 },
        Step::InitializeAsNotification,
        Step::Initialized,
        Step::ToolsList { id: 3 },
        Step::ModernToolsList { id: 4 },
        Step::CallEcho {
            id: 5,
            text: "hola\nmundo".into(),
        },
        Step::CallUnknownTool { id: 6 },
        Step::Ping { id: 7 },
        Step::PingStrId { id: "s".into() },
        Step::UnknownMethod { id: 8 },
        Step::ParamsArray { id: 9 },
        Step::NullId,
        Step::FloatId,
        Step::HugeId,
        Step::MissingJsonrpc { id: 10 },
        Step::WrongJsonrpc { id: 11 },
        Step::MethodNotString { id: 12 },
        Step::EmptyBatch,
        Step::Batch {
            items: vec![Step::Ping { id: 13 }, Step::Initialized],
        },
        Step::Cancelled { request_id: 14 },
        Step::ClientResponse { id: 15 },
        Step::UnknownNotification,
        Step::Oversized { id: 16, bytes: 10 },
    ];
    for variant in 0..4 {
        v.push(Step::CallEchoBadArgs { id: 20, variant });
        v.push(Step::NonObject { variant });
        v.push(Step::Malformed { variant });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_for_every_op() {
        for s in all_examples() {
            let back = Step::from_json(&s.to_json()).unwrap();
            assert_eq!(back, s, "{}", s.op());
        }
    }

    #[test]
    fn render_is_a_single_line() {
        for s in all_examples() {
            let line = s.render();
            assert!(!line.contains(&b'\n'), "{}", s.op());
            assert!(!line.is_empty());
        }
    }

    #[test]
    fn valid_steps_render_parseable_json() {
        for s in all_examples() {
            let parsed = serde_json::from_slice::<Value>(&s.render());
            match s {
                Step::Malformed { .. } => assert!(parsed.is_err(), "{}", s.op()),
                _ => assert!(parsed.is_ok(), "{}", s.op()),
            }
        }
    }

    #[test]
    fn oversized_has_requested_padding() {
        let small = Step::Oversized { id: 1, bytes: 0 }.render().len();
        let big = Step::Oversized { id: 1, bytes: 1000 }.render().len();
        assert_eq!(big - small, 1000);
    }

    #[test]
    fn expectations() {
        assert_eq!(Step::Initialized.expect(), Expect::Nothing);
        assert_eq!(Step::Ping { id: 1 }.expect(), Expect::Reply);
        assert_eq!(
            Step::Batch {
                items: vec![Step::Initialized]
            }
            .expect(),
            Expect::Nothing
        );
        assert_eq!(Step::Malformed { variant: 1 }.expect(), Expect::Reply);
    }

    #[test]
    fn request_ids() {
        assert_eq!(
            Step::Oversized { id: 3, bytes: 1 }.request_id(),
            Some(json!(3))
        );
        assert_eq!(Step::Ping { id: 4 }.request_id(), Some(json!(4)));
        assert_eq!(Step::Initialized.request_id(), None);
        assert_eq!(Step::Malformed { variant: 2 }.request_id(), None);
    }

    #[test]
    fn unknown_op_is_rejected() {
        assert!(Step::from_json(&json!({ "op": "nope" })).is_err());
        assert!(Step::from_json(&json!({ "op": "ping" })).is_err());
    }
}
