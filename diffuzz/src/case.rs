//! Executing one case (a sequence of steps) against every target.

use std::thread;

use crate::oracle::{self, Finding};
use crate::step::Step;
use crate::target::{Session, StepObs, TargetSpec, Timing};

#[derive(Debug, Clone)]
pub struct CaseRun {
    pub names: Vec<String>,
    pub per_target: Vec<Vec<StepObs>>,
    pub endings: Vec<(String, bool)>,
    pub findings: Vec<Finding>,
}

fn run_one(spec: &TargetSpec, steps: &[Step], timing: &Timing) -> (Vec<StepObs>, (String, bool)) {
    let mut session = match Session::spawn(spec) {
        Ok(s) => s,
        Err(e) => {
            let dead = StepObs {
                exited: Some(format!("spawn-failed: {e}")),
                ..StepObs::default()
            };
            return (
                vec![dead; steps.len()],
                (format!("spawn-failed: {e}"), false),
            );
        }
    };
    let obs = steps.iter().map(|s| session.exchange(s, timing)).collect();
    let end = session.finish(timing);
    (obs, end)
}

/// Run `steps` against all targets in parallel (one process per target)
/// and evaluate the oracles.
pub fn run(targets: &[TargetSpec], steps: &[Step], timing: &Timing) -> CaseRun {
    let results: Vec<(Vec<StepObs>, (String, bool))> = thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .map(|t| scope.spawn(move || run_one(t, steps, timing)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("target thread"))
            .collect()
    });
    let names: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();
    let (per_target, endings): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    let findings = oracle::evaluate(&names, steps, &per_target, &endings);
    CaseRun {
        names,
        per_target,
        endings,
        findings,
    }
}
