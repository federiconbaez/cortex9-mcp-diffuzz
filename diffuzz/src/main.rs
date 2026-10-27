use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use mcp_diffuzz::corpus::{self, Case, FuzzConfig};
use mcp_diffuzz::gen::GenConfig;
use mcp_diffuzz::target::{TargetSpec, Timing};

const USAGE: &str = "\
mcp-diffuzz - stateful differential fuzzer for MCP servers over stdio

USAGE:
    mcp-diffuzz fuzz   --target NAME=CMD [--target NAME=CMD]... [options]
    mcp-diffuzz replay --target NAME=CMD [--target NAME=CMD]... CASE.json...

FUZZ OPTIONS:
    --seed N            PRNG seed (default 1)
    --cases N           number of generated cases (default 200)
    --time-limit S      stop after S seconds
    --max-len N         max generated steps after the lifecycle prefix (default 10)
    --max-oversize N    largest oversized line padding in bytes (default 16777216)
    --reply-ms N        reply timeout per request (default 2000)
    --out DIR           write minimized cases and summary.json to DIR
    --no-minimize       keep the full generated sequence

REPLAY exits with status 1 if a case no longer reproduces its signature.
";

struct Args {
    targets: Vec<TargetSpec>,
    rest: Vec<String>,
    opts: std::collections::HashMap<String, String>,
    flags: Vec<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        targets: Vec::new(),
        rest: Vec::new(),
        opts: Default::default(),
        flags: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--target" => a.targets.push(TargetSpec::parse(
                it.next().ok_or("--target needs a value")?,
            )?),
            "--no-minimize" => a.flags.push(x.clone()),
            o if o.starts_with("--") => {
                let v = it.next().ok_or(format!("{o} needs a value"))?;
                a.opts.insert(o.to_string(), v.clone());
            }
            _ => a.rest.push(x.clone()),
        }
    }
    if a.targets.is_empty() {
        return Err("at least one --target is required".into());
    }
    Ok(a)
}

fn num(a: &Args, k: &str, default: u64) -> Result<u64, String> {
    match a.opts.get(k) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("{k}: '{v}' is not a number")),
    }
}

fn timing(a: &Args) -> Result<Timing, String> {
    Ok(Timing {
        reply: Duration::from_millis(num(a, "--reply-ms", 2000)?),
        ..Timing::default()
    })
}

fn fuzz(a: &Args) -> Result<ExitCode, String> {
    let cfg = FuzzConfig {
        seed: num(a, "--seed", 1)?,
        cases: num(a, "--cases", 200)?,
        time_limit: a
            .opts
            .get("--time-limit")
            .map(|_| num(a, "--time-limit", 0).map(Duration::from_secs))
            .transpose()?,
        gen: GenConfig {
            max_len: num(a, "--max-len", 10)? as usize,
            max_oversize: num(a, "--max-oversize", 16 << 20)? as usize,
        },
        timing: timing(a)?,
        minimize: !a.flags.iter().any(|f| f == "--no-minimize"),
    };
    let out = a.opts.get("--out").map(PathBuf::from);
    let stats = corpus::fuzz(&a.targets, &cfg, |c| {
        eprintln!("new  {}  ({} steps)", c.signature, c.steps.len());
        if let Some(dir) = &out {
            if let Err(e) = c.save(dir) {
                eprintln!("cannot save case: {e}");
            }
        }
    });
    let summary = stats.to_json();
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&summary).expect("json") + "\n";
        std::fs::write(dir.join("summary.json"), text).map_err(|e| e.to_string())?;
    }
    println!("{}", serde_json::to_string_pretty(&summary).expect("json"));
    Ok(ExitCode::SUCCESS)
}

fn replay(a: &Args) -> Result<ExitCode, String> {
    if a.rest.is_empty() {
        return Err("replay needs at least one case file".into());
    }
    let t = timing(a)?;
    let mut failed = 0;
    for path in &a.rest {
        let c = Case::load(path.as_ref())?;
        let ok = corpus::reproduces(&a.targets, &c.steps, &c.signature, &t);
        println!(
            "{}  {}  {}",
            if ok { "REPRODUCED" } else { "GONE      " },
            path,
            c.signature
        );
        if !ok {
            failed += 1;
        }
    }
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("fuzz") => parse(&args[1..]).and_then(|a| fuzz(&a)),
        Some("replay") => parse(&args[1..]).and_then(|a| replay(&a)),
        Some("-h" | "--help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err(format!("unknown or missing command\n\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
