//! S1b of epic #31 (#33) — the inbound LISTENER: accept a delivered handoff (the version-stamped
//! envelope from #32) and enqueue it for the target cof (write the station queue + set `hasMail`).
//!
//! Pull, never push. This is the RECEIVER half of the cross-host delivery pair: the sender (#32)
//! opens a real channel (`ssh <host> …` / `tailscale ssh …` / `herdr machine <host> …`) and hands
//! the remote `comms-axi deliver <envelope>` verb this [`Envelope`]. The listener parses it,
//! enforces the VERSION-GATED handshake, and enqueues the payload onto the target station's LOCAL
//! queue (the S1 `send` contract: `<state-dir>/queues/<role>.json` + `<role>.hasmail`). It does
//! **NOT** inject eagerly — no `resolve_role`, no `send_event`, no harness session; the target
//! station pulls on idle (S2 `read`/`drain` + S3 wake). No mid-turn interrupt.
//!
//! **Transition rules (hard, from spec.md):**
//! 2. **VERSION-GATED handshake** — the listener speaks [`PROTOCOL_VERSION`]; an envelope speaking
//!    a different version is rejected LOUDLY before anything is enqueued — never a half-delivery.
//!    This is the receiver half of [`crate::delivery::handshake`] (#32's sender-side check was
//!    tautological until this listener actually validates the wire value).
//! 4. **NO teardown, preserve in-flight** — the envelope carries a STABLE effect id; the listener
//!    enqueues UNDER that id (not the local per-station sequence), so the journal's `prepare` dedup
//!    is keyed on it. A re-delivery of an already-journaled id is ACKNOWLEDGED but NOT re-enacted
//!    (the #35 idempotency guarantee: `Accepted.duplicate` is `true`), so a mid-swap re-send
//!    survives — no lost envelope, no double injection.
//!
//! **Sender attribution (honest gap).** The #32 envelope is `{version, effect_id, to, payload}` —
//!    it carries NO `from`/sender identity. The listener therefore records
//!    [`REMOTE_SENDER`] as the stream `source_id` rather than inventing a role. Adding an explicit
//!    `from` to the envelope is a follow-up; until then cross-host turns read as `from: "remote"`.

use std::path::Path;

use serde::Serialize;

use crate::delivery::{handshake, DeliveryError, Envelope, PROTOCOL_VERSION};
use crate::error::{Error, Result};
use crate::send::send_to_with_id;

/// The stream `source_id` recorded for a cross-host delivery until the envelope carries an
/// explicit `from` (a later slice of epic #31). The #32 envelope has no sender identity, so the
/// listener attributes the turn to this honest placeholder rather than inventing a role.
pub const REMOTE_SENDER: &str = "remote";

/// The receipt the inbound listener returns: what was accepted, for which role, and the resulting
/// `hasMail` state. `effect_id` is the envelope's STABLE id (the #35 dedup axis), not the local
/// per-station sequence the queue assigned. `duplicate` is `true` when the effect id was already
/// journaled (a re-delivery — acknowledged but NOT re-enacted, the idempotency guarantee).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Accepted {
    pub role: String,
    pub effect_id: String,
    pub queued: usize,
    pub has_mail: bool,
    pub duplicate: bool,
}

/// The inbound listener. Accepts a delivered envelope (the #32 wire JSON), enforces the
/// version-gated handshake (rejects a mismatch LOUDLY — never a half-delivery), then enqueues
/// `envelope.payload` onto `envelope.to`'s station queue and sets its `hasMail` flag. It does NOT
/// inject eagerly: the target station pulls on idle (S2/S3).
pub fn accept(envelope_json: &str, state_dir: &Path) -> Result<Accepted> {
    // Parse the envelope first — a malformed wire value is loud, never a silent no-op.
    let envelope: Envelope = serde_json::from_str(envelope_json).map_err(|e| {
        Error::operational(
            format!("deliver envelope is not valid JSON: {e}"),
            "DELIVER_BAD_ENVELOPE",
        )
    })?;

    // Version-gated handshake (transition rule 2): reject a mismatch BEFORE any channel/enqueue —
    // never a half-delivery. Reuses the canonical `handshake` primitive from #32's delivery layer.
    handshake(PROTOCOL_VERSION, envelope.version).map_err(|e| match e {
        DeliveryError::VersionMismatch { local, remote } => Error::operational(
            format!(
                "deliver handshake failed: listener speaks v{local}, envelope speaks v{remote} — \
                 rejecting, never half-delivers"
            ),
            "DELIVER_VERSION",
        ),
        // `handshake` only ever fails with `VersionMismatch`; defensive, unreachable.
        other => Error::operational(
            format!("deliver handshake failed: {other}"),
            "DELIVER_VERSION",
        ),
    })?;

    // Enqueue for the target cof under the envelope's STABLE effect id (the #35 dedup axis, kept
    // intact here). Pull, never push — the recipient reads on idle.
    let enqueued = send_to_with_id(
        state_dir,
        &envelope.to,
        REMOTE_SENDER,
        &envelope.payload,
        &envelope.effect_id,
    )?;

    Ok(Accepted {
        role: envelope.to,
        effect_id: enqueued.effect_id,
        queued: enqueued.queued,
        has_mail: enqueued.has_mail,
        duplicate: enqueued.duplicate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::stable_effect_id;
    use crate::send::{has_mail, load_queue};
    use serde_json::json;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "comms-axi-listener-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A well-formed v1 envelope exactly as #32's `plan` emits it.
    fn envelope_json(to: &str, payload: &str) -> String {
        serde_json::to_string(&Envelope {
            version: PROTOCOL_VERSION,
            effect_id: stable_effect_id(to, payload),
            to: to.to_string(),
            payload: payload.to_string(),
        })
        .unwrap()
    }

    #[test]
    fn accept_enqueues_and_sets_has_mail() {
        let dir = tmpdir("accept-enqueues");
        let r = accept(&envelope_json("cof", "done: PR #9"), &dir).unwrap();
        assert_eq!(r.role, "cof");
        assert_eq!(r.effect_id, stable_effect_id("cof", "done: PR #9"));
        assert_eq!(r.queued, 1);
        assert!(r.has_mail);

        // The payload landed on the target station's queue, and hasMail is set.
        assert!(has_mail(&dir, "cof"));
        let store = load_queue(&dir, "cof").unwrap();
        assert_eq!(store.len(), 1);
        let e = &store.stream.entries()[0];
        assert_eq!(e.payload, "done: PR #9");
        assert_eq!(e.source_id, REMOTE_SENDER);
    }

    #[test]
    fn accept_is_pull_never_push() {
        // The listener only ENQUEUES — it never drains/admits (no eager injection). After accept
        // the event is still pending (journal Prepared), hasMail still set, nothing consumed.
        let dir = tmpdir("accept-pull");
        accept(&envelope_json("cof", "hi"), &dir).unwrap();
        let store = load_queue(&dir, "cof").unwrap();
        assert_eq!(store.len(), 1, "still queued — not eagerly injected");
        assert!(
            has_mail(&dir, "cof"),
            "hasMail still set — the station pulls on idle"
        );
    }

    #[test]
    fn accept_rejects_version_mismatch_loudly() {
        let dir = tmpdir("accept-version");
        let bad = serde_json::to_string(&Envelope {
            version: 2,
            effect_id: stable_effect_id("cof", "x"),
            to: "cof".to_string(),
            payload: "x".to_string(),
        })
        .unwrap();
        let err = accept(&bad, &dir).unwrap_err();
        assert_eq!(err.code, "DELIVER_VERSION");
        assert!(
            err.message.contains("handshake failed"),
            "got: {}",
            err.message
        );
        // Nothing was enqueued, no marker was set — never a half-delivery.
        assert!(!has_mail(&dir, "cof"));
        assert!(load_queue(&dir, "cof").unwrap().is_empty());
    }

    #[test]
    fn accept_rejects_malformed_envelope() {
        let dir = tmpdir("accept-malformed");
        let err = accept("not json", &dir).unwrap_err();
        assert_eq!(err.code, "DELIVER_BAD_ENVELOPE");
        assert!(!has_mail(&dir, "cof"));
    }

    #[test]
    fn accept_is_per_station_no_cross_writes() {
        // `to` is the ONLY role that gets a queue entry / marker — the listener never writes
        // another station.
        let dir = tmpdir("accept-per-station");
        accept(&envelope_json("cof", "hi"), &dir).unwrap();
        assert!(has_mail(&dir, "cof"));
        assert!(!has_mail(&dir, "coordinator"));
        assert!(load_queue(&dir, "coordinator").unwrap().is_empty());
    }

    #[test]
    fn accept_appends_in_order_with_distinct_stable_ids() {
        let dir = tmpdir("accept-order");
        accept(&envelope_json("cof", "one"), &dir).unwrap();
        accept(&envelope_json("cof", "two"), &dir).unwrap();
        let store = load_queue(&dir, "cof").unwrap();
        let payloads: Vec<_> = store
            .stream
            .entries()
            .iter()
            .map(|e| e.payload.as_str())
            .collect();
        assert_eq!(payloads, vec!["one", "two"]);
        // Distinct payloads carry distinct stable effect ids.
        let a = stable_effect_id("cof", "one");
        let b = stable_effect_id("cof", "two");
        assert_ne!(a, b);
    }

    #[test]
    fn accept_result_serializes() {
        let dir = tmpdir("accept-serialize");
        let r = accept(&envelope_json("cof", "hi"), &dir).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["role"], "cof");
        assert_eq!(v["queued"], 1);
        assert_eq!(v["has_mail"], true);
        assert_eq!(v["effect_id"], json!(stable_effect_id("cof", "hi")));
        assert_eq!(v["duplicate"], json!(false));
    }

    #[test]
    fn accept_redelivery_is_idempotent_not_double_injected() {
        // The #35 idempotency guarantee, end-to-end: a re-delivery of the SAME envelope (same
        // STABLE effect id) is acknowledged but NOT re-enacted — queued stays 1 pre-drain, and
        // stays 0 post-drain (no double injection across the transition).
        let dir = tmpdir("accept-idempotent");
        let env = envelope_json("cof", "done: PR #9");

        // Fresh delivery: enqueued, not a duplicate.
        let first = accept(&env, &dir).unwrap();
        assert!(!first.duplicate);
        assert_eq!(first.queued, 1);
        assert!(first.has_mail);

        // Re-delivery BEFORE drain: deduped — queued stays 1, flagged duplicate.
        let second = accept(&env, &dir).unwrap();
        assert!(second.duplicate, "pre-drain re-delivery is a duplicate");
        assert_eq!(second.queued, 1, "never double-injects");

        // Drain (S2 read/inject): the turn is consumed.
        crate::read::read(&dir, "cof", true).unwrap();
        assert!(!has_mail(&dir, "cof"));

        // Re-delivery AFTER drain (the mid-swap re-send): deduped — NOT re-enacted.
        let third = accept(&env, &dir).unwrap();
        assert!(third.duplicate, "post-drain re-delivery is a duplicate");
        assert_eq!(third.queued, 0, "a consumed effect is never re-enacted");
        assert!(!third.has_mail);
        assert!(!has_mail(&dir, "cof"));
    }

    #[test]
    fn accept_dedup_keys_on_stable_id_not_local_seq() {
        // Two distinct payloads share NO stable id; each is enqueued (not falsely deduped). The
        // dedup axis is the FNV-1a stable id, NOT the local per-station `{role}-{seq}` sequence.
        let dir = tmpdir("accept-stable-key");
        let a = accept(&envelope_json("cof", "one"), &dir).unwrap();
        let b = accept(&envelope_json("cof", "two"), &dir).unwrap();
        assert!(!a.duplicate);
        assert!(!b.duplicate);
        assert_ne!(a.effect_id, b.effect_id);
        let store = load_queue(&dir, "cof").unwrap();
        assert_eq!(store.len(), 2, "distinct stable ids both enqueued");
        // The stable id is what the journal recorded, not `cof-0`/`cof-1`.
        assert_eq!(store.stream.entries()[0].effect_id, a.effect_id);
        assert_eq!(store.stream.entries()[1].effect_id, b.effect_id);
    }
}
