//! `comms-axi` — the comm virtualization CLI (lib + bin crate).
//!
//! `resolve`/`emit`/`report` (the canary plane) plus `send` (S1 deferred publish) and
//! `read`/`drain` (S2 pull) are implemented.

use std::path::PathBuf;
use std::process::ExitCode;

use comms_axi::event::{send_event, Event};
use comms_axi::read::read as read_queue;
use comms_axi::report::report;
use comms_axi::resolve::resolve_role;
use comms_axi::send::{send_to, Enqueued};
use comms_axi::wake::{idle_tick, WakeAction};
use serde_json::{json, Value};

const USAGE: &str = "\
comms-axi — surface-agnostic agent messaging plane

USAGE:
    comms-axi resolve <role> [--run <id>] [--surface <hint>] [--record <path>] [--json]
    comms-axi emit <role> <event> [--run <id>] [--surface <hint>] [--record <path>] [--json]
    comms-axi report <event> [--run <id>] [--surface <hint>] [--record <path>] [--json]
    comms-axi send <role> <event> [--state-dir <path>] [--json]
    comms-axi read <role> [--all] [--state-dir <path>] [--json]
    comms-axi drain <role> [--state-dir <path>] [--json]
    comms-axi wake <role> [--state-dir <path>] [--json]

FLAGS:
    --run <id>       run id (default: \"default\")
    --surface <hint> R4 surface hint (cos_surface); a stale hint errors loudly
    --record <path>  load the fleet/run record JSON from <path> (else empty record)
    --json           print the result (StationBinding / Dispatch / Report / Enqueued / ReadResult) as JSON
    --json           print the result (StationBinding / Dispatch / Report / Enqueued / WakeAction) as JSON
    -h, --help       this help

report resolves the SEAT's own binding (role from $CF_ROLE) and delegates to the adapter's
return channel — herdr-axi report on herdr, cmux-axi status on cmux.

send enqueues an event to a recipient role's LOCAL queue and sets its hasMail flag (deferred
publish) — it never resolves-and-fires into a live session. The sender is $CF_ROLE; the queue
lives under --state-dir (default $COMMS_AXI_STATE_DIR, else $CF_COF_HOME/.omp/state, else
$HOME/.omp/state).

read pulls a role's queue on demand (bounded by default, --all drains everything); drain is an
alias for read --all. Both clear hasMail when the queue empties and render the drained events as
one injectable turn.
wake is the hasMail-gated idle tick: file-stat only — re-wake if the station has pending mail,
else a silent no-op (zero tokens). Non-idle never wakes.
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
        "send" => cmd_send(&args[1..]),
        "read" => cmd_read(&args[1..]),
        "drain" => cmd_drain(&args[1..]),
        "wake" => cmd_wake(&args[1..]),
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

fn cmd_send(args: &[String]) -> Result<ExitCode, String> {
    if args.len() < 2 {
        return Err("send requires <role> <event>; try `comms-axi --help`".to_string());
    }
    let role = &args[0];
    let payload = &args[1];

    let mut state_dir: Option<String> = None;
    let mut json_out = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--state-dir" => {
                i += 1;
                state_dir = Some(args.get(i).cloned().ok_or("--state-dir needs a value")?);
            }
            "--json" => json_out = true,
            other => return Err(format!("unknown flag {other:?}; try `comms-axi --help`")),
        }
        i += 1;
    }

    // The seat's own role is the sender; the queue lives under the station's state dir.
    let sender = std::env::var("CF_ROLE")
        .map_err(|_| "send needs the sender's own role: set $CF_ROLE".to_string())?;
    let dir = resolve_state_dir(state_dir.as_deref());

    let enq: Enqueued = send_to(&dir, role, &sender, payload).map_err(|e| e.to_string())?;

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&enq).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "send -> {}: queued={} effect_id={} has_mail={} (deferred; recipient pulls on idle)",
            role, enq.queued, enq.effect_id, enq.has_mail
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_read(args: &[String]) -> Result<ExitCode, String> {
    read_impl(args, false)
}

fn cmd_drain(args: &[String]) -> Result<ExitCode, String> {
    read_impl(args, true)
}

fn read_impl(args: &[String], force_all: bool) -> Result<ExitCode, String> {
    if args.is_empty() {
        return Err("read requires <role>; try `comms-axi --help`".to_string());
    }
    let role = &args[0];

    let mut all = force_all;
    let mut state_dir: Option<String> = None;
    let mut json_out = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--all" => all = true,
            "--state-dir" => {
                i += 1;
                state_dir = Some(args.get(i).cloned().ok_or("--state-dir needs a value")?);
            }
            "--json" => json_out = true,
            other => return Err(format!("unknown flag {other:?}; try `comms-axi --help`")),
        }
        i += 1;
    }

    let dir = resolve_state_dir(state_dir.as_deref());
    let result = read_queue(&dir, role, all).map_err(|e| e.to_string())?;

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "read -> {}: {} event(s), {} remaining, has_mail={}",
            role,
            result.events.len(),
            result.remaining,
            result.has_mail
        );
        for e in &result.events {
            println!("  - {}: {}", e.from, e.text);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_wake(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() {
        return Err("wake requires <role>; try `comms-axi --help`".to_string());
    }
    let role = &args[0];

    let mut state_dir: Option<String> = None;
    let mut json_out = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--state-dir" => {
                i += 1;
                state_dir = Some(args.get(i).cloned().ok_or("--state-dir needs a value")?);
            }
            "--json" => json_out = true,
            other => return Err(format!("unknown flag {other:?}; try `comms-axi --help`")),
        }
        i += 1;
    }

    let dir = resolve_state_dir(state_dir.as_deref());
    // The idle-tick wake: file-stat only. The role's own idle state is true on a heartbeat tick.
    let action: WakeAction = idle_tick(&dir, role, true);

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&action).map_err(|e| e.to_string())?
        );
    } else {
        println!("wake -> {}: {}", role, action.as_str());
    }
    Ok(ExitCode::SUCCESS)
}

/// The state dir: `--state-dir` wins, else `$COMMS_AXI_STATE_DIR`, else
/// `$CF_COF_HOME/.omp/state`, else `$HOME/.omp/state`.
///
/// `$CF_COF_HOME` is cf's tracked CoS home — the single source of truth cf's heartbeat reader
/// (#497) anchors on. When it is set, the comms-axi WRITER (`send`/`read`/`wake`) must land its
/// `hasMail` marker on the same path the READER stats, or the pull loop is perma-dead on machines
/// where `$HOME` != `$CF_COF_HOME`.
fn resolve_state_dir(flag: Option<&str>) -> PathBuf {
    if let Some(p) = flag {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("COMMS_AXI_STATE_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("CF_COF_HOME") {
        if !p.trim().is_empty() {
            return PathBuf::from(p).join(".omp").join("state");
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".omp").join("state")
}

#[cfg(test)]
mod tests {
    use super::resolve_state_dir;

    fn clear(keys: &[&str]) {
        for k in keys {
            std::env::remove_var(k);
        }
    }

    fn set(key: &str, val: &str) {
        std::env::set_var(key, val);
    }

    // Env mutation is process-global and would race under the parallel test runner, so all five
    // precedence cases live in ONE test — they run sequentially and the vars are cleared after.
    #[test]
    fn state_dir_precedence_with_cf_cof_home() {
        // 1. --state-dir wins over every env var.
        set("CF_COF_HOME", "/cf/home");
        set("COMMS_AXI_STATE_DIR", "/env/state");
        set("HOME", "/home/user");
        assert_eq!(
            resolve_state_dir(Some("/flag/dir")),
            std::path::PathBuf::from("/flag/dir")
        );

        // 2. $COMMS_AXI_STATE_DIR beats $CF_COF_HOME.
        assert_eq!(
            resolve_state_dir(None),
            std::path::PathBuf::from("/env/state")
        );

        // 3. $CF_COF_HOME set → <CF_COF_HOME>/.omp/state (the cf reader #497 path).
        clear(&["COMMS_AXI_STATE_DIR"]);
        assert_eq!(
            resolve_state_dir(None),
            std::path::PathBuf::from("/cf/home/.omp/state")
        );

        // 4. $CF_COF_HOME empty → not a home; fall through to $HOME.
        set("CF_COF_HOME", "");
        assert_eq!(
            resolve_state_dir(None),
            std::path::PathBuf::from("/home/user/.omp/state")
        );

        // 5. $CF_COF_HOME unset → $HOME fallback.
        clear(&["CF_COF_HOME"]);
        assert_eq!(
            resolve_state_dir(None),
            std::path::PathBuf::from("/home/user/.omp/state")
        );

        clear(&["COMMS_AXI_STATE_DIR", "CF_COF_HOME", "HOME"]);
    }
}
