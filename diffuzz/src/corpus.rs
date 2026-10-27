//! Regression cases on disk and the fuzzing loop that produces them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::case;
use crate::gen::{self, GenConfig};
use crate::minimize::ddmin;
use crate::norm;
use crate::oracle::Finding;
use crate::rng::Rng;
use crate::step::Step;
use crate::target::{TargetSpec, Timing};

/// A minimized, reproducible finding.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub signature: String,
    pub kind: String,
    pub target: Option<String>,
    pub op: String,
    pub steps: Vec<Step>,
    /// Abstract observation of every step, per target, when it was saved.
    pub observed: BTreeMap<String, Vec<String>>,
    pub note: String,
}

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(40).collect()
}

impl Case {
    pub fn file_name(&self) -> String {
        format!(
            "{}-{}-{:08x}.json",
            slug(&self.kind),
            slug(&self.op),
            fnv(&self.signature)
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "format": 1,
            "signature": self.signature,
            "kind": self.kind,
            "target": self.target,
            "op": self.op,
            "note": self.note,
            "steps": self.steps.iter().map(Step::to_json).collect::<Vec<_>>(),
            "observed": self.observed,
        })
    }

    pub fn from_json(v: &Value) -> Result<Case, String> {
        if v["format"] != 1 {
            return Err("unsupported case format".into());
        }
        let s = |k: &str| {
            v[k].as_str()
                .map(String::from)
                .ok_or(format!("missing {k}"))
        };
        let observed = v["observed"]
            .as_object()
            .ok_or("missing observed")?
            .iter()
            .map(|(k, arr)| {
                let list = arr
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                (k.clone(), list)
            })
            .collect();
        Ok(Case {
            signature: s("signature")?,
            kind: s("kind")?,
            target: v["target"].as_str().map(String::from),
            op: s("op")?,
            note: v["note"].as_str().unwrap_or("").to_string(),
            steps: v["steps"]
                .as_array()
                .ok_or("missing steps")?
                .iter()
                .map(Step::from_json)
                .collect::<Result<_, _>>()?,
            observed,
        })
    }

    pub fn load(path: &Path) -> Result<Case, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Case::from_json(&v).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(self.file_name());
        let mut text = serde_json::to_string_pretty(&self.to_json()).expect("json");
        text.push('\n');
        std::fs::write(&path, text)?;
        Ok(path)
    }
}

/// Whether running `steps` reproduces `signature`.
pub fn reproduces(
    targets: &[TargetSpec],
    steps: &[Step],
    signature: &str,
    timing: &Timing,
) -> bool {
    case::run(targets, steps, timing)
        .findings
        .iter()
        .any(|f| f.signature() == signature)
}

/// Minimize the steps that produced `finding` and package them as a [`Case`].
pub fn minimize(
    targets: &[TargetSpec],
    steps: &[Step],
    finding: &Finding,
    timing: &Timing,
) -> Case {
    let sig = finding.signature();
    let prefix: Vec<Step> = match finding.step {
        Some(i) => steps[..=i].to_vec(),
        None => steps.to_vec(),
    };
    let base = if reproduces(targets, &prefix, &sig, timing) {
        prefix
    } else {
        steps.to_vec()
    };
    let min = ddmin(&base, |cand| reproduces(targets, cand, &sig, timing));
    let run = case::run(targets, &min, timing);
    let observed = run
        .names
        .iter()
        .zip(&run.per_target)
        .map(|(n, obs)| {
            let abs = min.iter().zip(obs).map(|(s, o)| norm::step(s, o)).collect();
            (n.clone(), abs)
        })
        .collect();
    Case {
        signature: sig,
        kind: finding.kind.label(),
        target: finding.target.clone(),
        op: finding.op.clone(),
        steps: min,
        observed,
        note: String::new(),
    }
}

#[derive(Debug, Clone)]
pub struct FuzzConfig {
    pub seed: u64,
    pub cases: u64,
    pub time_limit: Option<Duration>,
    pub gen: GenConfig,
    pub timing: Timing,
    pub minimize: bool,
}

#[derive(Debug, Default, Clone)]
pub struct FuzzStats {
    pub cases_run: u64,
    pub steps_run: u64,
    pub raw_findings: u64,
    pub unique: Vec<Case>,
    pub elapsed: Duration,
}

impl FuzzStats {
    pub fn to_json(&self) -> Value {
        let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
        for c in &self.unique {
            *by_kind.entry(c.kind.clone()).or_default() += 1;
        }
        json!({
            "cases_run": self.cases_run,
            "steps_run": self.steps_run,
            "raw_findings": self.raw_findings,
            "unique_signatures": self.unique.len(),
            "unique_by_kind": by_kind,
            "elapsed_s": self.elapsed.as_secs_f64(),
        })
    }
}

/// The main loop: generate, run, deduplicate by signature, minimize.
pub fn fuzz(targets: &[TargetSpec], cfg: &FuzzConfig, mut on_new: impl FnMut(&Case)) -> FuzzStats {
    let start = Instant::now();
    let mut stats = FuzzStats::default();
    let mut seen = std::collections::BTreeSet::new();
    let mut rng = Rng::new(cfg.seed);
    for _ in 0..cfg.cases {
        if cfg.time_limit.is_some_and(|t| start.elapsed() >= t) {
            break;
        }
        let steps = gen::case(&mut rng, &cfg.gen);
        let run = case::run(targets, &steps, &cfg.timing);
        stats.cases_run += 1;
        stats.steps_run += steps.len() as u64;
        stats.raw_findings += run.findings.len() as u64;
        for f in &run.findings {
            let sig = f.signature();
            if !seen.insert(sig.clone()) {
                continue;
            }
            let c = if cfg.minimize {
                minimize(targets, &steps, f, &cfg.timing)
            } else {
                Case {
                    signature: sig,
                    kind: f.kind.label(),
                    target: f.target.clone(),
                    op: f.op.clone(),
                    steps: steps.clone(),
                    observed: BTreeMap::new(),
                    note: String::new(),
                }
            };
            on_new(&c);
            stats.unique.push(c);
        }
    }
    stats.elapsed = start.elapsed();
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_roundtrip() {
        let mut observed = BTreeMap::new();
        observed.insert("a".to_string(), vec!["ok(empty,id=req)".to_string()]);
        let c = Case {
            signature: "divergence|*|ping|a=x;b=y".into(),
            kind: "divergence".into(),
            target: None,
            op: "ping".into(),
            steps: crate::step::all_examples(),
            observed,
            note: "n".into(),
        };
        assert_eq!(Case::from_json(&c.to_json()).unwrap(), c);
        assert!(c.file_name().starts_with("divergence-ping-"));
        assert!(c.file_name().ends_with(".json"));
    }

    #[test]
    fn slugs() {
        assert_eq!(
            slug("invariant:response-without-id"),
            "invariant-response-without-id"
        );
        assert_eq!(slug("call-echo-bad-args-3"), "call-echo-bad-args-3");
    }
}
