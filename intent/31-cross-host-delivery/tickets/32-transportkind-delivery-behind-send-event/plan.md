# Plan: ticket #32 — TransportKind delivery behind send_event (ssh/tailscale/herdr-machine)
Epic: #31. Date: 2026-09-15. Status: done.

## Files that change
- src/adapter.rs — `TransportKind::is_cross_host()` helper (ssh/tailscale/herdr-machine are the
  deliverable transports; local-socket/sbox-remote are not).
- src/delivery.rs (new) — the cross-host delivery layer: `PROTOCOL_VERSION`, `Envelope`, `Delivery`,
  `DeliveryError`, `handshake`, `plan`, `deliver_event`, `stable_effect_id`.
- src/event.rs — `send_event` returns `SendOutcome::{Render, Deliver}`; cross-host path gated behind
  `CF_COMMS_MODE=comms` + a remote transport.
- src/main.rs — `cmd_emit` opens the REAL channel (spawns the transport argv) on the Deliver path.
- src/lib.rs — register `delivery` module.

## Order of work
1. `TransportKind::is_cross_host()`.
2. `delivery.rs` (envelope + plan + handshake + gate), pure core.
3. `send_event` → `SendOutcome`; default-off render stays byte-stable.
4. `cmd_emit` spawns the channel on Deliver.
5. Tests, fmt, clippy, smoke, evidence, PR.

## Transition rules baked in (hard)
- **ADDITIVE + default-off**: cross-host delivery only when `CF_COMMS_MODE=comms`; unset = herdr
  render-only `Dispatch`, byte-stable vs today.
- **VERSION-GATED handshake**: envelope carries `PROTOCOL_VERSION`; `handshake(local, remote)` fails
  loudly on mismatch; never half-delivers (fall back to herdr render).
- **Herdr REMAINS the carrier**: `herdr-machine` transport rides `herdr` (`herdr machine <host> …`);
  comms-axi does NOT fork a second comm path, revive cmux, or replace herdr.
- **Canary-first / no-teardown**: transport selection is per-binding (`binding.transport`), never a
  seat-typed surface; no flag-day cutover, no eager push (injection is pull-on-idle, #34).

## Validation Strategy
- Unit: `handshake` match/mismatch; `plan` gate-off → Disabled; local-socket/sbox-remote → NoChannel;
  ssh/tailscale/herdr-machine → correct argv + version-stamped envelope; `gate`/`stable_effect_id`
  determinism.
- `send_event`: default (gate off) → `Render(Dispatch)` byte-identical to today; gate on + ssh →
  `Deliver` with real argv.
- fmt/clippy clean; smoke `emit` in comms mode emits the channel argv (no real host in CI).
- Coverage target: every `plan`/`handshake`/gate branch.

## Proof
`cargo test` green (count in evidence), `cargo fmt --check` + `cargo clippy --workspace -- -D
warnings` exit 0; evidence under `evidence/validate-*.txt`.

## Risks
- cf receipt half (codefactory #515) is OUT of scope; comms-axi owns delivery + listener + injection.
- Do NOT spec/build the listener (#33) or effect-id dedup (#35) here — #32 is the sender delivery.
