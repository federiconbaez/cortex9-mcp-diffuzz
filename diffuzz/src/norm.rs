//! Normalization: turn raw replies into abstract observations that can be
//! compared across implementations.
//!
//! Things that legitimately differ between servers are erased: error
//! messages, server names, capabilities, instructions, extra tools, JSON
//! key order, tool error texts and the decoration added by `2026-07-28`
//! (`resultType`, `_meta`, `ttlMs`, `cacheScope`). What is kept: whether a
//! reply exists, result vs. error, the error code, the negotiated protocol
//! version, whether `echo` is listed, the echoed text, `isError`, and the
//! shape of the `id` member.

use serde_json::Value;

use crate::step::{Expect, Step};
use crate::target::StepObs;

/// Shape of the `id` member of a reply relative to the request.
fn id_shape(reply: &Value, request_id: Option<&Value>) -> &'static str {
    match reply.get("id") {
        None => "absent",
        Some(Value::Null) => "null",
        Some(id) => match request_id {
            Some(req) if req == id => "req",
            _ => "other",
        },
    }
}

fn result_shape(r: &Value) -> String {
    if let Some(v) = r.get("protocolVersion").and_then(Value::as_str) {
        return format!("init({v})");
    }
    if let Some(tools) = r.get("tools").and_then(Value::as_array) {
        let echo = tools.iter().any(|t| t["name"] == "echo");
        return format!("tools(echo={echo})");
    }
    if r.get("content").is_some() {
        if r["isError"] == Value::Bool(true) {
            return "tool-error".into();
        }
        let text: String = r["content"][0]["text"]
            .as_str()
            .unwrap_or("?")
            .chars()
            .take(32)
            .collect();
        return format!("text({text})");
    }
    match r {
        Value::Object(m) => {
            let rest = m
                .keys()
                .any(|k| !matches!(k.as_str(), "_meta" | "resultType" | "ttlMs" | "cacheScope"));
            if rest {
                "obj".into()
            } else {
                "empty".into()
            }
        }
        _ => "non-object".into(),
    }
}

/// Abstract form of one reply line; `request_id` is the id of the request
/// that caused it, if any.
pub fn reply(line: &[u8], request_id: Option<&Value>) -> String {
    let v: Value = match serde_json::from_slice(line) {
        Ok(v) => v,
        Err(_) => return "garbage".into(),
    };
    one(&v, request_id)
}

fn one(v: &Value, request_id: Option<&Value>) -> String {
    match v {
        Value::Array(items) => {
            let mut parts: Vec<String> = items.iter().map(|i| one(i, None)).collect();
            parts.sort();
            format!("batch[{}]", parts.join(","))
        }
        Value::Object(m) => {
            let id = id_shape(v, request_id);
            if let Some(e) = m.get("error") {
                let code = e.get("code").and_then(Value::as_i64).unwrap_or(0);
                format!("err({code},id={id})")
            } else if let Some(r) = m.get("result") {
                format!("ok({},id={id})", result_shape(r))
            } else {
                "not-a-response".into()
            }
        }
        _ => "non-object".into(),
    }
}

/// Abstract observation of a whole step for one target.
pub fn step(step: &Step, obs: &StepObs) -> String {
    if obs.exited.is_some() {
        return "EXITED".into();
    }
    let request_id = step.request_id();
    let mut parts: Vec<String> = obs
        .lines
        .iter()
        .map(|l| reply(l, request_id.as_ref()))
        .collect();
    parts.sort();
    if parts.is_empty() {
        return match step.expect() {
            Expect::Nothing => "silent".into(),
            Expect::Reply => "NO-REPLY".into(),
        };
    }
    parts.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obs(lines: &[&str]) -> StepObs {
        StepObs {
            lines: lines.iter().map(|l| l.as_bytes().to_vec()).collect(),
            ..StepObs::default()
        }
    }

    #[test]
    fn erases_server_specific_details() {
        let s = Step::Initialize {
            id: 1,
            version: "2025-11-25".into(),
        };
        let a = obs(&[
            r#"{"id":1,"jsonrpc":"2.0","result":{"protocolVersion":"2025-11-25","serverInfo":{"name":"a"},"capabilities":{"tools":{}}}}"#,
        ]);
        let b = obs(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{},"protocolVersion":"2025-11-25","serverInfo":{"name":"b"},"instructions":"x"}}"#,
        ]);
        assert_eq!(step(&s, &a), step(&s, &b));
        assert_eq!(step(&s, &a), "ok(init(2025-11-25),id=req)");
    }

    #[test]
    fn error_messages_are_ignored_but_codes_kept() {
        let s = Step::Ping { id: 4 };
        let a = obs(&[r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"x"}}"#]);
        let b = obs(&[r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"y"}}"#]);
        let c = obs(&[r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32602,"message":"x"}}"#]);
        assert_eq!(step(&s, &a), step(&s, &b));
        assert_ne!(step(&s, &a), step(&s, &c));
    }

    #[test]
    fn id_shapes() {
        let s = Step::Ping { id: 4 };
        let req = s.request_id();
        let r = |l: &str| reply(l.as_bytes(), req.as_ref());
        assert_eq!(
            r(r#"{"jsonrpc":"2.0","error":{"code":1}}"#),
            "err(1,id=absent)"
        );
        assert_eq!(
            r(r#"{"jsonrpc":"2.0","id":null,"error":{"code":1}}"#),
            "err(1,id=null)"
        );
        assert_eq!(
            r(r#"{"jsonrpc":"2.0","id":5,"result":{}}"#),
            "ok(empty,id=other)"
        );
        assert_eq!(
            r(r#"{"jsonrpc":"2.0","id":4,"result":{}}"#),
            "ok(empty,id=req)"
        );
    }

    #[test]
    fn modern_decoration_is_erased() {
        let r = json!({"resultType":"complete","_meta":{},"ttlMs":0,"cacheScope":"public"});
        assert_eq!(result_shape(&r), "empty");
        let tools = json!({"tools":[{"name":"calc"},{"name":"echo"}],"resultType":"complete"});
        assert_eq!(result_shape(&tools), "tools(echo=true)");
    }

    #[test]
    fn tool_results() {
        assert_eq!(
            result_shape(&json!({"content":[{"type":"text","text":"hi"}],"isError":false})),
            "text(hi)"
        );
        assert_eq!(
            result_shape(&json!({"content":[{"type":"text","text":"boom"}],"isError":true})),
            "tool-error"
        );
    }

    #[test]
    fn batches_are_order_insensitive() {
        let a = reply(
            br#"[{"id":1,"result":{}},{"id":2,"error":{"code":-1}}]"#,
            None,
        );
        let b = reply(
            br#"[{"id":2,"error":{"code":-1}},{"id":1,"result":{}}]"#,
            None,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn missing_and_unexpected_replies() {
        assert_eq!(step(&Step::Ping { id: 1 }, &obs(&[])), "NO-REPLY");
        assert_eq!(step(&Step::Initialized, &obs(&[])), "silent");
        assert_eq!(step(&Step::Ping { id: 1 }, &obs(&["nope"])), "garbage");
        let dead = StepObs {
            exited: Some("signal(9)".into()),
            ..StepObs::default()
        };
        assert_eq!(step(&Step::Ping { id: 1 }, &dead), "EXITED");
    }
}
