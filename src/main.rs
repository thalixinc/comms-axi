//! `comms-axi` — the comm virtualization CLI (lib + bin crate).
//!
//! `resolve` (R2) and `emit` (R1/R5) are implemented; `report` (#12) follows.

use std::process::ExitCode;

use comms_axi::event::{send_event, Event};
use comms_axi::report::report;
use comms_axi::resolve::resolve_role;
use serde_json::{json, Value};

const USAGE: &str = "\
comms-axi — surface-agnostic agent messaging plane

USAGE:
    comms-axi resolve <role> [--run <id>] [--surface <hint>] [--record <path>] [--json]
    comms-axi emit <role> <event> [--run <id>] [--surface <hint>] [--record <path>] [--json]
    comms-axi report <event> [--run <id>] [--surface <hint>] [--record <path>] [--json]

FLAGS:
    --run <id>       run id (default: \"default\")
    --surface <hint> R4 surface hint (cos_surface); a stale hint errors loudly
    --record <path>  load the fleet/run record JSON from <path> (else empty record)
    --json           print the result (StationBinding / Dispatch / Report) as JSON
    -h, --help       this help

report resolves the SEAT's own binding (role from $CF_ROLE) and delegates to the adapter's
return channel — herdr-axi report on herdr, cmux-axi status on cmux.
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
        "emit" => cmd_emit(&args[1..]),
        "report" => cmd_report(&args[1..]),
        other => Err(format!("unknown verb {other:?}; try `comms-axi --help`")),
    }
}

/// The shared `--run` / `--surface` / `--record` / `--json` flags (R4 hint, G5 record).
struct CommonOpts {
    run: String,
    surface: Option<String>,
    record_path: Option<String>,
    json: bool,
}

fn parse_common(args: &[String]) -> Result<CommonOpts, String> {
    let mut opts = CommonOpts {
        run: "default".to_string(),
        surface: None,
        record_path: None,
        json: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--run" => {
                i += 1;
                opts.run = args.get(i).cloned().ok_or("--run needs a value")?;
            }
            "--surface" => {
                i += 1;
                opts.surface = Some(args.get(i).cloned().ok_or("--surface needs a value")?);
            }
            "--record" => {
                i += 1;
                opts.record_path = Some(args.get(i).cloned().ok_or("--record needs a value")?);
            }
            "--json" => opts.json = true,
            other => return Err(format!("unknown flag {other:?}; try `comms-axi --help`")),
        }
        i += 1;
    }
    Ok(opts)
}

/// Load the fleet/run record from `--record` (else empty), then layer the `--surface` hint into
/// `cos_surface` (R4).
fn load_record(record_path: Option<&str>, surface: Option<&str>) -> Result<Value, String> {
    let mut record = match record_path {
        Some(path) => {
            let bytes =
                std::fs::read(path).map_err(|e| format!("cannot read record {path:?}: {e}"))?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|e| format!("record {path:?} is not valid JSON: {e}"))?
        }
        None => json!({}),
    };
    if let Some(hint) = surface {
        record["cos_surface"] = json!(hint);
    }
    Ok(record)
}

fn cmd_resolve(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() {
        return Err("resolve requires a <role>; try `comms-axi --help`".to_string());
    }
    let role = &args[0];
    let opts = parse_common(&args[1..])?;
    let record = load_record(opts.record_path.as_deref(), opts.surface.as_deref())?;

    let binding = resolve_role(role, &opts.run, &record).map_err(|e| e.to_string())?;

    if opts.json {
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

fn cmd_emit(args: &[String]) -> Result<ExitCode, String> {
    if args.len() < 2 {
        return Err("emit requires <role> <event>; try `comms-axi --help`".to_string());
    }
    let role = &args[0];
    let payload = &args[1];
    let opts = parse_common(&args[2..])?;
    let record = load_record(opts.record_path.as_deref(), opts.surface.as_deref())?;

    let binding = resolve_role(role, &opts.run, &record).map_err(|e| e.to_string())?;
    let dispatch = send_event(&binding, &Event::new(role, payload)).map_err(|e| e.to_string())?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&dispatch).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "emit -> {}: adapter={} tool={} target={} text={:?}",
            role,
            dispatch.adapter.as_str(),
            dispatch.tool,
            dispatch.target,
            dispatch.text
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_report(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() {
        return Err("report requires <event>; try `comms-axi --help`".to_string());
    }
    let payload = &args[0];
    let opts = parse_common(&args[1..])?;
    let record = load_record(opts.record_path.as_deref(), opts.surface.as_deref())?;

    // The seat's own role comes from the runtime context ($CF_ROLE), never a seat-typed token.
    let role = std::env::var("CF_ROLE")
        .map_err(|_| "report needs the seat's own role: set $CF_ROLE".to_string())?;

    let rep = report(&role, &opts.run, &record, payload).map_err(|e| e.to_string())?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&rep).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "report -> {}: adapter={} tool={} text={:?}",
            role,
            rep.adapter.as_str(),
            rep.tool,
            rep.text
        );
    }
    Ok(ExitCode::SUCCESS)
}
