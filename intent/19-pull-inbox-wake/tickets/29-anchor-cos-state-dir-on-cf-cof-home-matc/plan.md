# Plan: ticket #29 — anchor CoS state_dir on CF_COF_HOME (writer/reader share one path)
Epic: #19. Date: 2026-09-14. Status: done.

## Files that change
- src/main.rs — add the `$CF_COF_HOME/.omp/state` tier to `resolve_state_dir` (between
  `$COMMS_AXI_STATE_DIR` and `$HOME/.omp/state`); update `--help`; add a serial precedence test.

## Order of work
1. Insert the CF_COF_HOME tier into resolve_state_dir.
2. Add a single serial test covering all five precedence cases (flag > COMMS_AXI_STATE_DIR >
   CF_COF_HOME > HOME; empty/unset CF_COF_HOME fall through).
3. Test, fmt, clippy, smoke test (CF_COF_HOME set → marker under <CF_COF_HOME>/.omp/state), evidence.

## Validation Strategy
- Unit (`cargo test --bin comms-axi`): precedence — flag wins; COMMS_AXI_STATE_DIR beats CF_COF_HOME;
  CF_COF_HOME set → `<CF_COF_HOME>/.omp/state`; empty CF_COF_HOME → HOME; unset → HOME.
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `CF_COF_HOME=<tmp> comms-axi send …` → `hasMail` marker at
  `<tmp>/.omp/state/queues/<role>.hasmail` (the exact path cf#497 stats).
- Coverage target: all five precedence branches in resolve_state_dir.

## Proof
`cargo test` → 88 passed (87 lib + 1 bin), 0 failed. `cargo fmt --check` + `cargo clippy --workspace
-- -D warnings` exit 0. Smoke: marker under `$CF_COF_HOME/.omp/state/queues/`.

## Risks
- One env tier added, nothing else changed — release-cut discipline.
- The "should cf bump dep to f7234ad" question is FLAGGED BACK on codefactory#497 (not resolved here).
