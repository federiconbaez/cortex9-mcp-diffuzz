//! Oracles: per-target JSON-RPC 2.0 invariants plus cross-target
//! disagreement.

use serde_json::Value;

use crate::norm;
use crate::step::{Expect, Step};
use crate::target::StepObs;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The abstract observations of the targets disagree.
    Divergence,
    /// A target stopped answering a request.
    Hang,
    /// A target exited while stdin was still open, or exited uncleanly.
    Crash,
    /// A target broke a JSON-RPC 2.0 rule on its own.
    Invariant(&'static str),
}

impl Kind {
    pub fn label(&self) -> String {
        match self {
            Kind::Divergence => "divergence".into(),
            Kind::Hang => "hang".into(),
            Kind::Crash => "crash".into(),
            Kind::Invariant(n) => format!("invariant:{n}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    /// Target at fault (`None` for divergences).
    pub target: Option<String>,
    /// Index of the step that triggered it (`None`: at shutdown).
    pub step: Option<usize>,
    pub op: String,
    /// Abstract observation per target, in target order.
    pub observed: Vec<(String, String)>,
}

/// Coarse form of an abstract observation for signatures: batch contents
/// and echoed texts are dropped, so the same root cause seen with different
/// payloads deduplicates to one signature.
pub fn coarse(obs: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in obs.chars() {
        if depth > 0 {
            match c {
                '[' | '(' => depth += 1,
                ']' | ')' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(c);
                    }
                }
                _ => {}
            }
            continue;
        }
        out.push(c);
        if out.ends_with("batch[") || out.ends_with("text(") {
            out.push_str("..");
            depth = 1;
        }
    }
    out
}

impl Finding {
    /// Stable identity used for deduplication and regression checks.
    /// Ids, step positions, batch contents and echoed texts are not part of
    /// it.
    pub fn signature(&self) -> String {
        let obs: Vec<String> = self
            .observed
            .iter()
            .map(|(t, o)| format!("{t}={}", coarse(o)))
            .collect();
        format!(
            "{}|{}|{}|{}",
            self.kind.label(),
            self.target.as_deref().unwrap_or("*"),
            self.op,
            obs.join(";")
        )
    }
}

/// Per-target invariant checks for one step.
pub fn invariants(step: &Step, obs: &StepObs) -> Vec<Kind> {
    let mut out = Vec::new();
    if obs.exited.is_some() {
        out.push(Kind::Crash);
        return out;
    }
    if obs.timed_out {
        out.push(Kind::Hang);
    }
    let request = step.value();
    let mut replies = 0;
    for line in &obs.lines {
        let v: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => {
                out.push(Kind::Invariant("non-json-output"));
                continue;
            }
        };
        let items: Vec<&Value> = match &v {
            Value::Array(a) => a.iter().collect(),
            other => vec![other],
        };
        for item in items {
            if item.get("result").is_none() && item.get("error").is_none() {
                // Server-initiated requests/notifications are allowed.
                continue;
            }
            replies += 1;
            if item.get("jsonrpc") != Some(&Value::String("2.0".into())) {
                out.push(Kind::Invariant("jsonrpc-member"));
            }
            if item.get("id").is_none() {
                out.push(Kind::Invariant("response-without-id"));
            }
            if item.get("result").is_some() && item.get("error").is_some() {
                out.push(Kind::Invariant("result-and-error"));
            }
            if let (Some(req), false) = (&request, matches!(step, Step::Batch { .. })) {
                if item.get("result").is_some() && item.get("id") != req.get("id") {
                    out.push(Kind::Invariant("id-mismatch"));
                }
            }
        }
    }
    match step.expect() {
        Expect::Nothing if replies > 0 => out.push(Kind::Invariant("reply-to-notification")),
        Expect::Reply if replies > 1 && !matches!(step, Step::Batch { .. }) => {
            out.push(Kind::Invariant("multiple-replies"))
        }
        _ => {}
    }
    out.sort();
    out.dedup();
    out
}

/// All findings for one executed case.
///
/// `per_target[t][i]` is target `t`'s observation of step `i`;
/// `endings[t]` is how target `t` ended after stdin was closed.
pub fn evaluate(
    names: &[String],
    steps: &[Step],
    per_target: &[Vec<StepObs>],
    endings: &[(String, bool)],
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut crashed = vec![false; names.len()];
    let mut disturbed = false;
    for (i, step) in steps.iter().enumerate() {
        let abs: Vec<(String, String)> = names
            .iter()
            .zip(per_target)
            .map(|(n, o)| (n.clone(), norm::step(step, &o[i])))
            .collect();
        for (t, name) in names.iter().enumerate() {
            if crashed[t] {
                continue;
            }
            for kind in invariants(step, &per_target[t][i]) {
                if kind == Kind::Crash {
                    crashed[t] = true;
                }
                out.push(Finding {
                    kind,
                    target: Some(name.clone()),
                    step: Some(i),
                    op: step.op(),
                    observed: vec![(name.clone(), abs[t].1.clone())],
                });
            }
        }
        // After a crash or a hang, later replies may be misattributed: the
        // per-target finding is kept, cross-target comparison stops.
        if names.len() > 1 && !disturbed && abs.iter().any(|(_, o)| o != &abs[0].1) {
            out.push(Finding {
                kind: Kind::Divergence,
                target: None,
                step: Some(i),
                op: step.op(),
                observed: abs.clone(),
            });
        }
        if abs.iter().any(|(_, o)| o == "EXITED" || o == "NO-REPLY") {
            disturbed = true;
        }
    }
    for (t, name) in names.iter().enumerate() {
        let (how, killed) = &endings[t];
        if !crashed[t] && (*killed || how != "exit(0)") {
            out.push(Finding {
                kind: Kind::Crash,
                target: Some(name.clone()),
                step: None,
                op: if *killed {
                    "eof-ignored"
                } else {
                    "eof-unclean-exit"
                }
                .into(),
                observed: vec![(name.clone(), how.clone())],
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(lines: &[&str]) -> StepObs {
        StepObs {
            lines: lines.iter().map(|l| l.as_bytes().to_vec()).collect(),
            ..StepObs::default()
        }
    }

    #[test]
    fn clean_reply_has_no_invariant_findings() {
        let s = Step::Ping { id: 1 };
        assert!(invariants(&s, &obs(&[r#"{"jsonrpc":"2.0","id":1,"result":{}}"#])).is_empty());
    }

    #[test]
    fn detects_rule_breaks() {
        let s = Step::Ping { id: 1 };
        let k = invariants(&s, &obs(&[r#"{"error":{"code":-32600}}"#]));
        assert!(k.contains(&Kind::Invariant("jsonrpc-member")));
        assert!(k.contains(&Kind::Invariant("response-without-id")));
        let two = obs(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        ]);
        assert!(invariants(&s, &two).contains(&Kind::Invariant("multiple-replies")));
        let wrong = obs(&[r#"{"jsonrpc":"2.0","id":2,"result":{}}"#]);
        assert!(invariants(&s, &wrong).contains(&Kind::Invariant("id-mismatch")));
        let n = invariants(
            &Step::Initialized,
            &obs(&[r#"{"jsonrpc":"2.0","id":null,"result":{}}"#]),
        );
        assert!(n.contains(&Kind::Invariant("reply-to-notification")));
        assert!(invariants(&s, &obs(&["zzz"])).contains(&Kind::Invariant("non-json-output")));
    }

    #[test]
    fn hang_and_crash() {
        let s = Step::Ping { id: 1 };
        let hang = StepObs {
            timed_out: true,
            ..StepObs::default()
        };
        assert_eq!(invariants(&s, &hang), vec![Kind::Hang]);
        let dead = StepObs {
            exited: Some("signal(11)".into()),
            ..StepObs::default()
        };
        assert_eq!(invariants(&s, &dead), vec![Kind::Crash]);
    }

    #[test]
    fn divergence_and_signature_ignore_ids() {
        let names = vec!["a".to_string(), "b".to_string()];
        let mk = |id: i64| {
            let steps = vec![Step::Ping { id }];
            let a = vec![obs(&[&format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{{}}}}"#
            )])];
            let b = vec![obs(&[&format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601}}}}"#
            )])];
            let ends = vec![("exit(0)".to_string(), false); 2];
            evaluate(&names, &steps, &[a, b], &ends)
        };
        let f1 = mk(1);
        let f2 = mk(99);
        assert_eq!(f1.len(), 1);
        assert_eq!(f1[0].kind, Kind::Divergence);
        assert_eq!(f1[0].signature(), f2[0].signature());
    }

    #[test]
    fn coarse_drops_payloads() {
        assert_eq!(
            coarse("batch[ok(empty,id=other),ok(text(a),id=other)]"),
            "batch[..]"
        );
        assert_eq!(coarse("ok(text(hi),id=req)"), "ok(text(..),id=req)");
        assert_eq!(coarse("err(-32600,id=absent)"), "err(-32600,id=absent)");
    }

    #[test]
    fn unclean_exit_is_a_crash() {
        let names = vec!["a".to_string()];
        let f = evaluate(&names, &[], &[vec![]], &[("exit(1)".into(), false)]);
        assert_eq!(f[0].kind, Kind::Crash);
        assert_eq!(f[0].op, "eof-unclean-exit");
        let f = evaluate(&names, &[], &[vec![]], &[("signal(9)".into(), true)]);
        assert_eq!(f[0].op, "eof-ignored");
    }
}
