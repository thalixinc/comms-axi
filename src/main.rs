//! `comms-axi` — the comm virtualization CLI (lib + bin crate).
//!
//! Only the `resolve` verb exists in this slice (R2); `emit` (#11) and `report` (#12) follow.

use std::process::ExitCode;

use comms_axi::resolve::{resolve_role, ResolveError};
use serde_json::{json, Value};

const USAGE: &str = "\
comms-axi — surface-agnostic agent messaging plane

USAGE:
    comms-axi resolve <role> [--run <id>] [--surface <hint>] [--record <path>] [--json]

FLAGS:
    --run <id>       run id (default: \"default\")
    --surface <hint> R4 surface hint (cos_surface); a stale hint errors loudly
    --record <path>  load the fleet/run record JSON from <path> (else empty record)
    --json           print the StationBinding as JSON
    -h, --help       this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("comms-axi: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }

    match args[0].as_str() {
        "resolve" => cmd_resolve(&args[1..]),
        other => Err(format!("unknown verb {other:?}; try `comms-axi --help`")),
    }
}

fn cmd_resolve(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() {
        return Err("resolve requires a <role>; try `comms-axi --help`".to_string());
    }
    let role = &args[0];

    let mut run = "default".to_string();
    let mut surface: Option<String> = None;
    let mut record_path: Option<String> = None;
    let mut json_out = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--run" => {
                i += 1;
                run = args.get(i).cloned().ok_or("--run needs a value")?;
            }
            "--surface" => {
                i += 1;
                surface = Some(args.get(i).cloned().ok_or("--surface needs a value")?);
            }
            "--record" => {
                i += 1;
                record_path = Some(args.get(i).cloned().ok_or("--record needs a value")?);
            }
            "--json" => json_out = true,
            other => return Err(format!("unknown flag {other:?}; try `comms-axi --help`")),
        }
        i += 1;
    }

    let mut record = match record_path {
        Some(path) => {
            let bytes =
                std::fs::read(&path).map_err(|e| format!("cannot read record {path:?}: {e}"))?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|e| format!("record {path:?} is not valid JSON: {e}"))?
        }
        None => json!({}),
    };
    if let Some(hint) = surface {
        record["cos_surface"] = json!(hint);
    }

    let binding = resolve_role(role, &run, &record).map_err(resolve_err_to_string)?;

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&binding).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "{}: adapter={} host={} transport={} surface_ref={} session_id={} route={}/{}@{}",
            role,
            binding.adapter.as_str(),
            binding.host.name,
            binding.transport.as_str(),
            binding.surface_ref,
            binding.session_id,
            binding.route.host,
            binding.route.surface,
            binding.route.generation,
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn resolve_err_to_string(e: ResolveError) -> String {
    e.to_string()
}
