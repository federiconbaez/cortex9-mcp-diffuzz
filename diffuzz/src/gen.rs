//! Stateful case generator.
//!
//! Cases usually start with a lifecycle prefix (`initialize`, then
//! `notifications/initialized`) so that the interesting states are reached,
//! and then mix valid traffic with malformed, out-of-order and duplicated
//! messages. Ids are mostly fresh but are sometimes reused on purpose.

use crate::rng::Rng;
use crate::step::{Step, VERSIONS};

#[derive(Debug, Clone, Copy)]
pub struct GenConfig {
    pub max_len: usize,
    /// Largest padding for oversized lines, in bytes (16 MiB by default:
    /// rmcp 3.8.0 drops stdio lines longer than 16 MiB).
    pub max_oversize: usize,
}

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            max_len: 10,
            max_oversize: 16 << 20,
        }
    }
}

struct Ids {
    next: i64,
    used: Vec<i64>,
}

impl Ids {
    fn take(&mut self, rng: &mut Rng) -> i64 {
        if !self.used.is_empty() && rng.chance(1, 8) {
            return *rng.pick(&self.used);
        }
        let id = self.next;
        self.next += 1;
        self.used.push(id);
        id
    }
}

const TEXTS: &[&str] = &[
    "hola",
    "",
    "ñandú ✓",
    "line\nbreak",
    "\u{0}",
    "{\"json\":1}",
];

fn simple(rng: &mut Rng, ids: &mut Ids) -> Step {
    match rng.below(5) {
        0 => Step::Ping { id: ids.take(rng) },
        1 => Step::ToolsList { id: ids.take(rng) },
        2 => Step::CallEcho {
            id: ids.take(rng),
            text: rng.pick(TEXTS).to_string(),
        },
        3 => Step::UnknownMethod { id: ids.take(rng) },
        _ => Step::Initialized,
    }
}

fn any_step(rng: &mut Rng, ids: &mut Ids, cfg: &GenConfig) -> Step {
    match rng.below(30) {
        0..=2 => Step::Ping { id: ids.take(rng) },
        3..=4 => Step::ToolsList { id: ids.take(rng) },
        5..=7 => Step::CallEcho {
            id: ids.take(rng),
            text: rng.pick(TEXTS).to_string(),
        },
        8 => Step::CallEchoBadArgs {
            id: ids.take(rng),
            variant: rng.below(4) as u8,
        },
        9 => Step::CallUnknownTool { id: ids.take(rng) },
        10 => Step::Initialize {
            id: ids.take(rng),
            version: rng.pick(VERSIONS).to_string(),
        },
        11 => Step::InitializeNoParams { id: ids.take(rng) },
        12 => Step::InitializeAsNotification,
        13 => Step::Initialized,
        14 => Step::ModernToolsList { id: ids.take(rng) },
        15 => Step::UnknownMethod { id: ids.take(rng) },
        16 => Step::ParamsArray { id: ids.take(rng) },
        17 => rng
            .pick(&[Step::NullId, Step::FloatId, Step::HugeId])
            .clone(),
        18 => Step::PingStrId {
            id: rng.pick(&["", "a", "1"]).to_string(),
        },
        19 => match rng.below(3) {
            0 => Step::MissingJsonrpc { id: ids.take(rng) },
            1 => Step::WrongJsonrpc { id: ids.take(rng) },
            _ => Step::MethodNotString { id: ids.take(rng) },
        },
        20 => Step::NonObject {
            variant: rng.below(4) as u8,
        },
        21..=22 => Step::Malformed {
            variant: rng.below(4) as u8,
        },
        23 => Step::EmptyBatch,
        24..=25 => {
            let n = 1 + rng.below(3) as usize;
            Step::Batch {
                items: (0..n).map(|_| simple(rng, ids)).collect(),
            }
        }
        26 => Step::Cancelled {
            request_id: ids.take(rng),
        },
        27 => Step::ClientResponse { id: ids.take(rng) },
        28 => Step::UnknownNotification,
        _ => {
            let sizes = [64 << 10, 1 << 20, cfg.max_oversize];
            Step::Oversized {
                id: ids.take(rng),
                bytes: *rng.pick(&sizes),
            }
        }
    }
}

/// Generate one case.
pub fn case(rng: &mut Rng, cfg: &GenConfig) -> Vec<Step> {
    let mut ids = Ids {
        next: 1,
        used: Vec::new(),
    };
    let mut steps = Vec::new();
    if rng.chance(3, 4) {
        steps.push(Step::Initialize {
            id: ids.take(rng),
            version: rng.pick(VERSIONS).to_string(),
        });
        if rng.chance(4, 5) {
            steps.push(Step::Initialized);
        }
    }
    let len = 1 + rng.below(cfg.max_len.max(1) as u64) as usize;
    for _ in 0..len {
        steps.push(any_step(rng, &mut ids, cfg));
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_a_seed() {
        let cfg = GenConfig::default();
        let a: Vec<_> = (0..20).map(|i| case(&mut Rng::new(i), &cfg)).collect();
        let b: Vec<_> = (0..20).map(|i| case(&mut Rng::new(i), &cfg)).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn lengths_are_bounded_and_nonempty() {
        let cfg = GenConfig {
            max_len: 5,
            ..GenConfig::default()
        };
        let mut rng = Rng::new(11);
        for _ in 0..500 {
            let c = case(&mut rng, &cfg);
            assert!(!c.is_empty() && c.len() <= 7, "{}", c.len());
        }
    }

    #[test]
    fn covers_the_vocabulary() {
        let cfg = GenConfig::default();
        let mut rng = Rng::new(5);
        let mut ops = std::collections::BTreeSet::new();
        for _ in 0..2000 {
            for s in case(&mut rng, &cfg) {
                ops.insert(s.op());
            }
        }
        for want in [
            "initialize",
            "initialized",
            "ping",
            "tools-list",
            "call-echo",
            "malformed-0",
            "batch",
            "empty-batch",
            "oversized",
            "null-id",
            "modern-tools-list",
            "cancelled",
            "client-response",
        ] {
            assert!(ops.contains(want), "missing {want}");
        }
    }

    #[test]
    fn ids_are_sometimes_reused() {
        let cfg = GenConfig::default();
        let mut rng = Rng::new(9);
        let dup = (0..300).any(|_| {
            let ids: Vec<_> = case(&mut rng, &cfg)
                .iter()
                .filter_map(|s| s.value().and_then(|v| v.get("id").and_then(|i| i.as_i64())))
                .collect();
            let mut sorted = ids.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len() < ids.len()
        });
        assert!(dup);
    }
}
