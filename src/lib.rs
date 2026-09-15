//! comms-axi — surface-agnostic agent messaging plane (event-to-role over `resolve_binding`).
//!
//! The seat's only comm surface is an **event addressed to a role**; the runtime resolves who,
//! how, and where — across hosts, not within one (founder amendment). `resolve` is the single
//! resolution source (R2), reading the fleet/host registry live (G5) and layering the R4 surface
//! precedence over it.

pub mod adapter;
pub mod delivery;
pub mod error;
pub mod event;
pub mod fleet;
pub mod inbox;
pub mod journal;
pub mod read;
pub mod report;
pub mod resolve;
pub mod scheduler;
pub mod send;
pub mod stream;
pub mod wake;
