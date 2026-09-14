# Spec amendment — comms-axi is FLEET-WIDE (remote-aware), not local-only

Founder decision (2026-09-13): **comms-axi must route across machines**, not just within one host.
The agents of a given role may live on any host in the fleet (this Mac, Thalixs-MBP, the omniroute
node, the EC2 control plane, sbox containers), reached over tailscale / ssh / herdr-machine /
sbox-remote.

## What changes in the spec

### 1. `StationBinding` gains a host + transport dimension

Current (host-blind):
```
StationBinding { adapter, surface_ref, session_id, route }
```

Amended (fleet-wide):
```
StationBinding {
    adapter:   AdapterKind,        // cmux | herdr | sbox (unchanged — the SURFACE)
    host:      HostRef,            // NEW — which machine the role lives on (e.g. tailscale name / ssh target / "local")
    transport: TransportKind,      // NEW — local-socket | tailscale | ssh | herdr-machine | sbox-remote
    surface_ref: String,           // the surface id ON that host
    session_id:  String,           // the session id ON that host
    route:       Route,            // the live handle, now host-qualified (not just a local handle)
}
```

### 2. `resolve_role` becomes host-aware

Resolution order for "who is `coordinator` and where are they":
1. **run/station record** → which host owns that station (the fleet record gains a `host` field).
2. **transport** → derived from host reachability (tailscale name present → tailscale; ssh target →
   ssh; local → local socket).
3. **adapter + surface** → as today, but scoped to that host.

The seat's API is UNCHANGED: `comms-axi emit <role> <event>` — remote-vs-local is invisible to the
seat. Only the resolver sees host/transport.

### 3. A host registry backs the resolution

A single fleet/host registry (extending the existing `fleet`/resolver record) is the source of truth
for: host name → transport → reachability → which stations live there. `resolve_role` reads it live
(truthful active-run migration, same as G5), never a static host list.

### 4. Transport is an adapter concern, not a new plane

comms-axi does NOT re-implement ssh/tailscale/sbox. It selects a transport and delegates to the
adapter that owns it (herdr-machine → herdr, sbox-remote → sbox, tailscale/ssh → the host's local
comms-axi-adapter running there). The neutral seam (R5) is preserved: still one resolution source,
one emit primitive, no second comm path.

## What this does NOT change

- Seat API (`emit` / `report` / `resolve`) — unchanged.
- Admission safety (CoS owns when a message enters context; no producer writes into the human terminal).
- Not-a-broker / not-a-cmux-revival boundaries.
- Hard-cutover delete-no-shims.
- G1–G5 modules move as-is (they're delivery substrate, host-agnostic).

## Scope for the first build

Local-first is fine *as long as the contract is host-aware from day one*: the `StationBinding`
carries `host` + `transport` from the start (even if the first transport implemented is `local`),
so remote transports (tailscale/ssh/herdr-machine/sbox) are ADDITIVE adapters, not a redesign.

## Relates to existing parked remote work

- `#420` surface-aware `cf machine` (herdr-machine remote) — becomes comms-axi's `herdr-machine`
  transport.
- `sbox` remote carrier — becomes the `sbox-remote` transport.
- herdr-axi `C→D` remote qualification — the `tailscale`/`ssh` transport.
These are the transport ADAPTERS comms-axi selects; none is comms-axi's own concern to reimplement.
