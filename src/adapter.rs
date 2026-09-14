//! The binding contract (R2/R5): the types `resolve_role` emits, host + transport-qualified.
//!
//! comms-axi routes ACROSS machines, not within one host (founder amendment). A role's agents may
//! live on any host in the fleet, reached over tailscale / ssh / herdr-machine / sbox-remote. The
//! seat's API is unchanged — only the resolver sees host/transport. The full `MESSAGING`
//! SURFACE→ADAPTER map (the #463 fold) lands in #11; here lives the binding shape itself.

use serde::Serialize;
use std::str::FromStr;

/// The transport adapters a binding may select (the SURFACE). `Cmux` is the legacy default;
/// `Sbox` is a station bound to a sandbox environment; `Herdr` is the transport of record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    Cmux,
    Sbox,
    Herdr,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::Cmux => "cmux",
            AdapterKind::Sbox => "sbox",
            AdapterKind::Herdr => "herdr",
        }
    }
}

impl FromStr for AdapterKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cmux" => Ok(AdapterKind::Cmux),
            "sbox" => Ok(AdapterKind::Sbox),
            "herdr" => Ok(AdapterKind::Herdr),
            _ => Err(format!("unknown adapter {s:?}")),
        }
    }
}

/// How a message reaches the host a role lives on. Only `LocalSocket` is built now; the remote
/// variants are NAMED (so the contract is host-aware from day one) but not yet implemented —
/// additive adapters later, never a redesign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    LocalSocket,
    Tailscale,
    Ssh,
    HerdrMachine,
    SboxRemote,
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TransportKind::LocalSocket => "local-socket",
            TransportKind::Tailscale => "tailscale",
            TransportKind::Ssh => "ssh",
            TransportKind::HerdrMachine => "herdr-machine",
            TransportKind::SboxRemote => "sbox-remote",
        }
    }
}

impl FromStr for TransportKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local-socket" => Ok(TransportKind::LocalSocket),
            "tailscale" => Ok(TransportKind::Tailscale),
            "ssh" => Ok(TransportKind::Ssh),
            "herdr-machine" => Ok(TransportKind::HerdrMachine),
            "sbox-remote" => Ok(TransportKind::SboxRemote),
            _ => Err(format!("unknown transport {s:?}")),
        }
    }
}

/// Which machine a role's station lives on (e.g. a tailscale name, an ssh target, or `local`).
/// Read from the fleet record's `host` field — never a static host list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostRef {
    pub name: String,
}

/// The fleet-resolved live handle, now host-qualified (founder amendment). Uniquely identifies
/// WHERE the binding lives — host + surface on that host + the observed-handle generation the run
/// actually used (G5 truthful stamp). Read from the fleet record, never derived statically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    pub host: String,
    pub surface: String,
    pub generation: u64,
}

/// A resolved station binding (fleet-wide shape, R2). The seat never sees this; producers consume
/// it. `adapter` is the SURFACE, `host` + `transport` are the remote-aware dimension,
/// `surface_ref`/`session_id` are the ids ON that host, and `route` is the host-qualified handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StationBinding {
    pub adapter: AdapterKind,
    pub host: HostRef,
    pub transport: TransportKind,
    pub surface_ref: String,
    pub session_id: String,
    pub route: Route,
}
