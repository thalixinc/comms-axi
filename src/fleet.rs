//! The versioned fleet resolver (#440 Gate 5), relocated as-is from
//! `codefactory/src/team/fleet.rs` (re-homed in #13).
//!
//! ONE versioned resolution source supplies the fleet's effective execution config. The
//! review.md failure this repairs: the CoS/Team/neutral resolvers kept DIFFERENT defaults and
//! could silently hold a default that contradicted what a run actually used.
//!
//! Three invariants, enforced in code, not prose:
//!
//! 1. **Inherit, never replace** — an explicit override names only the keys it changes; every
//!    other key inherits the fleet default. An override is a partial profile, and `resolve`
//!    MERGES it over the default (never swaps the whole profile).
//! 2. **Immutable + observed-handle-stamped** — `resolve` produces an owned, frozen
//!    [`EffectiveProfile`] stamped with the resolver version AND the observed-handle generation
//!    the run actually used, so "what the run used" is by construction "what the record says".
//! 3. **Explicit legacy migration** — a legacy `cos_surface` value is migrated into the new
//!    default EXPLICITLY (`migrate_legacy_cos_surface`), with `herdr` as the default when it is
//!    absent. Migration is idempotent and never infers a value that can silently diverge.
//!
//! herdr is the transport of record; cmux is not re-introduced as a parallel return channel.
//! This module is PURE (no I/O): callers persist [`EffectiveProfile`] via its
//! [`EffectiveProfile::to_record`] / [`EffectiveProfile::from_record`] seam, but the resolver
//! itself never touches disk, so it cannot silently hold a different default than the record.

use serde_json::{json, Value};

/// The resolver revision. Every profile carries it, so a profile resolved by an older revision
/// is distinguishable from one resolved under a newer default/merge rule.
pub const FLEET_RESOLVER_VERSION: u64 = 1;

/// The fleet default execution config every team inherits. `herdr` is the transport of record;
/// the fields here are the BASE that overrides merge OVER — never a value an override replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetDefault {
    pub surface: String,
    pub harness: String,
    pub layout: String,
}

impl Default for FleetDefault {
    fn default() -> Self {
        FleetDefault {
            surface: "herdr".to_string(),
            harness: "omp".to_string(),
            layout: "3by2".to_string(),
        }
    }
}

/// An explicit team override: a PARTIAL profile. `None` means "inherit the fleet default for
/// this key"; `Some` means "change this one key". An override NEVER replaces the whole profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Override {
    pub surface: Option<String>,
    pub harness: Option<String>,
    pub layout: Option<String>,
}

/// The effective run profile: fully resolved (no inherited slot left open), frozen once built,
/// and stamped with the resolver version + the observed-handle generation the run used.
///
/// Immutability is by construction: there are no setters, and `resolve` returns an owned value
/// so a later fleet-default change cannot touch an already-active profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveProfile {
    pub surface: String,
    pub harness: String,
    pub layout: String,
    /// The resolver revision that produced this profile.
    pub resolver_version: u64,
    /// The observed-handle generation a run actually used — the truthful stamp that makes
    /// "record == actually-used" hold, never a guess.
    pub observed_generation: u64,
}

impl EffectiveProfile {
    /// Serialize to the durable record a launch persists. `observed_generation` is carried
    /// through verbatim so the record round-trips without losing the truthful stamp.
    pub fn to_record(&self) -> Value {
        json!({
            "surface": self.surface,
            "harness": self.harness,
            "layout": self.layout,
            "resolver_version": self.resolver_version,
            "observed_generation": self.observed_generation,
        })
    }

    /// Rehydrate a profile from its record. A record missing the stamp or a required key is a
    /// corrupt record (loud), never a silently-inferred profile.
    pub fn from_record(v: &Value) -> Result<Self, String> {
        let get = |k: &str| -> Result<String, String> {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("effective profile record is missing {k:?}"))
        };
        Ok(EffectiveProfile {
            surface: get("surface")?,
            harness: get("harness")?,
            layout: get("layout")?,
            resolver_version: v
                .get("resolver_version")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    "effective profile record is missing resolver_version".to_string()
                })?,
            observed_generation: v
                .get("observed_generation")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    "effective profile record is missing observed_generation".to_string()
                })?,
        })
    }
}

/// Resolve the effective run profile from the fleet default + an explicit override, stamped with
/// the resolver version and the observed-handle generation the run actually used.
///
/// The inherit-not-replace rule: each key falls back to the fleet default unless the override
/// names it. A PARTIAL override is therefore complete after resolution — there is no silent
/// half-resolved slot, and no second resolution path can disagree.
///
/// A KEY the override names but that is empty or whitespace-only is a partial/corrupt input: it
/// cannot be inherited (the override named it) and cannot be used (it is not a real surface/
/// harness/layout). Rather than silently emit a half-resolved profile, `resolve` reports the
/// mid-launch partial failure truthfully.
pub fn resolve(
    fleet: &FleetDefault,
    over: &Override,
    observed_generation: u64,
) -> Result<EffectiveProfile, String> {
    // A named-but-blank override value is the one partial input this pure seam can detect: it
    // is neither "inherit" (a key was named) nor a usable value. Report it; never fabricate a
    // blank effective key.
    for (key, v) in [
        ("surface", &over.surface),
        ("harness", &over.harness),
        ("layout", &over.layout),
    ] {
        if let Some(s) = v {
            if s.trim().is_empty() {
                return Err(format!(
                    "override key {key:?} is empty — a partial launch cannot name a blank {key}"
                ));
            }
        }
    }
    Ok(EffectiveProfile {
        surface: over
            .surface
            .clone()
            .unwrap_or_else(|| fleet.surface.clone()),
        harness: over
            .harness
            .clone()
            .unwrap_or_else(|| fleet.harness.clone()),
        layout: over.layout.clone().unwrap_or_else(|| fleet.layout.clone()),
        resolver_version: FLEET_RESOLVER_VERSION,
        observed_generation,
    })
}

/// Migrate a legacy `cos_surface` value into an explicit fleet default.
///
/// The legacy world defaulted to `omp`; the new world defaults to `herdr`. Migration is
/// EXPLICIT: a founder who pinned `omp` keeps `omp` (their intent is preserved, never silently
/// flipped to herdr); a founder who pinned `herdr` keeps `herdr`; an ABSENT value yields the
/// herdr default. Idempotent by construction (pure, same input → same output).
pub fn migrate_legacy_cos_surface(legacy: Option<&str>) -> FleetDefault {
    let mut f = FleetDefault::default(); // herdr default
    if let Some(s) = legacy {
        match s.trim() {
            "omp" | "herdr" => f.surface = s.trim().to_string(),
            // An unknown/legacy value is not a guess: it falls back to the herdr default but is
            // NOT silently recorded as a real surface — callers surface the unknown upstream.
            _ => {}
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default() -> FleetDefault {
        FleetDefault::default()
    }

    #[test]
    fn override_inherits_fleet_default_never_replaces() {
        let f = default();
        let over = Override {
            surface: Some("omp".to_string()),
            harness: None,
            layout: None,
        };
        let p = resolve(&f, &over, 3).unwrap();
        assert_eq!(p.surface, "omp", "the override key changed");
        assert_eq!(
            p.harness, "omp",
            "the unnamed harness inherited the default"
        );
        assert_eq!(p.layout, "3by2", "the unnamed layout inherited the default");
        assert_eq!(p.resolver_version, FLEET_RESOLVER_VERSION);
        assert_eq!(p.observed_generation, 3);
    }

    #[test]
    fn partial_override_resolves_to_a_complete_profile() {
        let f = default();
        for over in [
            Override::default(),
            Override {
                surface: Some("omp".into()),
                ..Override::default()
            },
            Override {
                harness: Some("codex".into()),
                ..Override::default()
            },
            Override {
                layout: Some("2by2".into()),
                ..Override::default()
            },
        ] {
            let p = resolve(&f, &over, 0).unwrap();
            assert!(
                !p.surface.is_empty() && !p.harness.is_empty() && !p.layout.is_empty(),
                "no key may be left empty after resolve: {p:?}"
            );
        }
    }

    #[test]
    fn stamped_profile_round_trips_through_the_record() {
        let p = resolve(
            &default(),
            &Override {
                layout: Some("2by2".into()),
                ..Override::default()
            },
            42,
        )
        .unwrap();
        let rec = p.to_record();
        let back = EffectiveProfile::from_record(&rec).unwrap();
        assert_eq!(back, p, "record == actual, including the generation stamp");
        assert_eq!(back.observed_generation, 42);
        assert_eq!(back.resolver_version, FLEET_RESOLVER_VERSION);
    }

    #[test]
    fn change_default_does_not_affect_an_active_profile() {
        let f_herdr = default();
        let active = resolve(&f_herdr, &Override::default(), 1).unwrap();
        let f_omp = FleetDefault {
            surface: "omp".into(),
            ..default()
        };
        let _later = resolve(&f_omp, &Override::default(), 2).unwrap();
        assert_eq!(
            active.surface, "herdr",
            "the active profile is untouched by the default flip"
        );
        assert_eq!(active.observed_generation, 1, "and its stamp is unchanged");
    }

    #[test]
    fn legacy_cos_surface_migrates_explicitly_with_herdr_default() {
        assert_eq!(
            migrate_legacy_cos_surface(None).surface,
            "herdr",
            "absent -> herdr default"
        );
        assert_eq!(migrate_legacy_cos_surface(Some("herdr")).surface, "herdr");
        assert_eq!(
            migrate_legacy_cos_surface(Some("omp")).surface,
            "omp",
            "a pinned omp is preserved, not flipped"
        );
        let a = migrate_legacy_cos_surface(Some("omp"));
        let b = migrate_legacy_cos_surface(Some("omp"));
        assert_eq!(a, b);

        let p = resolve(
            &migrate_legacy_cos_surface(Some("omp")),
            &Override::default(),
            0,
        )
        .unwrap();
        assert_eq!(p.surface, "omp");
        assert_eq!(p.harness, "omp");
        assert_eq!(p.layout, "3by2");
    }

    #[test]
    fn corrupt_record_is_loud_not_inferred() {
        assert!(EffectiveProfile::from_record(&json!({"surface": "herdr"})).is_err());
        assert!(EffectiveProfile::from_record(
            &json!({"surface": "herdr", "harness": "omp", "layout": "3by2", "resolver_version": 1})
        )
        .is_err(), "missing generation is corrupt");
    }

    #[test]
    fn mid_launch_partial_failure_is_truthfully_reported_not_silent_default() {
        let f = default();
        for over in [
            Override {
                surface: Some("".into()),
                ..Override::default()
            },
            Override {
                harness: Some("   ".into()),
                ..Override::default()
            },
            Override {
                layout: Some("\t".into()),
                ..Override::default()
            },
        ] {
            let r = resolve(&f, &over, 7);
            assert!(
                r.is_err(),
                "a named-but-blank override key must fail, not resolve: {over:?}"
            );
            let err = r.unwrap_err();
            assert!(
                err.contains("empty"),
                "the error must say the key is empty, got: {err:?}"
            );
        }
        let ok = resolve(
            &f,
            &Override {
                surface: Some("omp".into()),
                ..Override::default()
            },
            7,
        );
        assert!(ok.is_ok() && ok.unwrap().surface == "omp");
    }
}
