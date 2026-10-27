//! Running a target MCP server as a child process over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::step::{Expect, Step};

/// Longest output line kept in memory; longer lines are truncated.
pub const MAX_KEPT_LINE: usize = 1 << 20;

/// A server under test: a name and the command that starts it in stdio mode.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetSpec {
    pub name: String,
    pub argv: Vec<String>,
}

impl TargetSpec {
    /// Parse `NAME=COMMAND [ARGS...]` (arguments split on whitespace).
    pub fn parse(s: &str) -> Result<Self, String> {
        let (name, cmd) = s
            .split_once('=')
            .ok_or_else(|| format!("target '{s}' must look like NAME=COMMAND"))?;
        let argv: Vec<String> = cmd.split_whitespace().map(String::from).collect();
        if name.is_empty() || argv.is_empty() {
            return Err(format!("target '{s}' must look like NAME=COMMAND"));
        }
        Ok(Self {
            name: name.to_string(),
            argv,
        })
    }
}

/// Timeouts used while talking to a target.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How long to wait for the reply to a request.
    pub reply: Duration,
    /// How long to listen after a notification (which must get no reply).
    pub quiet: Duration,
    /// Extra listening time after the first reply line, to catch duplicates.
    pub grace: Duration,
    /// How long to wait for a clean exit after stdin is closed.
    pub exit: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reply: Duration::from_millis(2000),
            quiet: Duration::from_millis(150),
            grace: Duration::from_millis(30),
            exit: Duration::from_millis(3000),
        }
    }
}

/// What a target did in response to one step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StepObs {
    /// Output lines (raw bytes, truncated to [`MAX_KEPT_LINE`]).
    pub lines: Vec<Vec<u8>>,
    /// A reply was expected and none arrived in time.
    pub timed_out: bool,
    /// The process exited (or closed stdout) during this step.
    pub exited: Option<String>,
    /// Writing the step failed (broken pipe).
    pub write_failed: bool,
}

enum Out {
    Line(Vec<u8>),
    Eof,
}

/// A live child process.
pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Out>,
    dead: Option<String>,
}

fn describe(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(c), _) => format!("exit({c})"),
        (None, Some(s)) => format!("signal({s})"),
        _ => "unknown".into(),
    }
}

impl Session {
    pub fn spawn(spec: &TargetSpec) -> std::io::Result<Self> {
        let mut child = Command::new(&spec.argv[0])
            .args(&spec.argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                // Read at most MAX_KEPT_LINE bytes into memory, skip the rest.
                let n = (&mut r)
                    .take(MAX_KEPT_LINE as u64)
                    .read_until(b'\n', &mut line);
                match n {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(Out::Eof);
                        return;
                    }
                    Ok(_) => {
                        if line.last() != Some(&b'\n') && line.len() >= MAX_KEPT_LINE {
                            let mut sink = Vec::new();
                            if r.read_until(b'\n', &mut sink).unwrap_or(0) == 0 {
                                let _ = tx.send(Out::Line(line));
                                let _ = tx.send(Out::Eof);
                                return;
                            }
                        }
                        while matches!(line.last(), Some(b'\n' | b'\r')) {
                            line.pop();
                        }
                        if tx.send(Out::Line(line)).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            rx,
            dead: None,
        })
    }

    fn reap(&mut self, wait: Duration) -> String {
        let end = Instant::now() + wait;
        loop {
            if let Ok(Some(st)) = self.child.try_wait() {
                return describe(st);
            }
            if Instant::now() >= end {
                return "stdout-closed".into();
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Send one step and collect what comes back.
    pub fn exchange(&mut self, step: &Step, timing: &Timing) -> StepObs {
        let mut obs = StepObs::default();
        if let Some(d) = &self.dead {
            obs.exited = Some(d.clone());
            return obs;
        }
        let mut bytes = step.render();
        bytes.push(b'\n');
        let wrote = self
            .stdin
            .as_mut()
            .map(|w| w.write_all(&bytes).and_then(|_| w.flush()))
            .unwrap_or_else(|| Err(std::io::Error::other("stdin closed")));
        if wrote.is_err() {
            obs.write_failed = true;
        }
        let expect = step.expect();
        // Big lines take longer to parse: 100 ms extra per MiB.
        let extra = Duration::from_millis((bytes.len() as u64 >> 20) * 100);
        let mut deadline = Instant::now()
            + match expect {
                Expect::Nothing => timing.quiet,
                Expect::Reply => timing.reply + extra,
            };
        let mut satisfied = false;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match self.rx.recv_timeout(deadline - now) {
                Ok(Out::Line(l)) => {
                    if l.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    obs.lines.push(l);
                    if expect == Expect::Reply && !satisfied {
                        satisfied = true;
                        deadline = Instant::now() + timing.grace;
                    }
                }
                Ok(Out::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    let d = self.reap(Duration::from_millis(500));
                    self.dead = Some(d.clone());
                    obs.exited = Some(d);
                    return obs;
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        if expect == Expect::Reply && !satisfied {
            obs.timed_out = true;
        }
        obs
    }

    /// Close stdin and wait for the process to exit; kill it if it does not.
    /// Returns how it ended and whether it had to be killed.
    pub fn finish(mut self, timing: &Timing) -> (String, bool) {
        drop(self.stdin.take());
        if let Some(d) = self.dead.take() {
            let _ = self.child.kill();
            let _ = self.child.wait();
            return (d, false);
        }
        let end = Instant::now() + timing.exit;
        loop {
            if let Ok(Some(st)) = self.child.try_wait() {
                return (describe(st), false);
            }
            if Instant::now() >= end {
                let _ = self.child.kill();
                let st = self.child.wait().map(describe).unwrap_or_default();
                return (st, true);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target() {
        let t = TargetSpec::parse("a=/bin/x stdio --flag").unwrap();
        assert_eq!(t.name, "a");
        assert_eq!(t.argv, vec!["/bin/x", "stdio", "--flag"]);
        assert!(TargetSpec::parse("nocommand").is_err());
        assert!(TargetSpec::parse("=x").is_err());
        assert!(TargetSpec::parse("a=").is_err());
    }

    fn sh(script: &str) -> TargetSpec {
        TargetSpec {
            name: "sh".into(),
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        }
    }

    #[test]
    fn echo_server_replies_and_exits_cleanly() {
        // `cat` echoes every line back: a reply arrives for each request.
        let mut s = Session::spawn(&sh("exec cat")).unwrap();
        let o = s.exchange(&Step::Ping { id: 1 }, &Timing::default());
        assert_eq!(o.lines.len(), 1);
        assert!(!o.timed_out);
        assert_eq!(s.finish(&Timing::default()), ("exit(0)".into(), false));
    }

    #[test]
    fn silent_server_times_out() {
        let timing = Timing {
            reply: Duration::from_millis(200),
            ..Timing::default()
        };
        let mut s = Session::spawn(&sh("cat > /dev/null")).unwrap();
        let o = s.exchange(&Step::Ping { id: 1 }, &timing);
        assert!(o.timed_out && o.lines.is_empty());
    }

    #[test]
    fn crash_is_reported() {
        let mut s = Session::spawn(&sh("read l; exit 3")).unwrap();
        let o = s.exchange(&Step::Ping { id: 1 }, &Timing::default());
        assert_eq!(o.exited.as_deref(), Some("exit(3)"));
        let again = s.exchange(&Step::Ping { id: 2 }, &Timing::default());
        assert_eq!(again.exited.as_deref(), Some("exit(3)"));
    }

    #[test]
    fn stuck_process_is_killed_on_finish() {
        let timing = Timing {
            exit: Duration::from_millis(200),
            ..Timing::default()
        };
        let s = Session::spawn(&sh("trap '' TERM; while :; do sleep 1; done")).unwrap();
        let (_, killed) = s.finish(&timing);
        assert!(killed);
    }
}
