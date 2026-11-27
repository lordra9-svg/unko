# AGENTS.md

Instructions for coding agents working in Unko. Read `CONTRIBUTING.md` first; it is the authority. This file is the short version plus what an agent needs to run the project.

## The project

A multi-platform, multi-core emulator shell in Rust. The NES core (`unko-nes`) is the reference-parity core; `unko-snes` is in progress. The host crates are `unko-core`, `unko-rom`, `unko-cli`, and `unko-app`. Workspace layout:

- `crates/unko-core` — system-generic core contract
- `crates/unko-nes` — NES core (must not depend on `unko-core`)
- `crates/unko-nes-api`, `crates/unko-nes-host`, `crates/unko-nes-dev` — NES API, adapter, dev tooling
- `crates/unko-snes` — SNES core
- `crates/unko-rom`, `crates/unko-rom-manager` — ROM parsing and the standalone ROM manager
- `crates/unko-netplay` — netplay
- `crates/unko-cli`, `crates/unko-app` — CLI and desktop app
- `xtask` — repository tasks
- `docs/` — architecture, accuracy ledgers, roadmap, platform notes

## Non-negotiables

- **No ROMs, BIOS images, or copyrighted firmware** may be committed. Tests that need public test ROMs look under `test-roms/` and must skip cleanly when it is absent.
- **No GPL source** (MesenCE, RustyNES, Mednafen, Dolphin, gopher64) may be copied, translated, vendored, or linked. They are behaviour oracles only.
- ares (ISC) may be adapted with attribution. Record every adaptation in `crates/unko-nes/PROVENANCE.md` and keep the notices in `NOTICE` / `licenses/ares-LICENSE.txt`.
- **Keep accuracy claims honest.** Do not raise an accuracy badge and do not remove a divergence from `docs/accuracy-divergences.md` or `docs/snes-accuracy-divergences.md` without a passing test or hardware capture.
- Do not bump the workspace version or edit release metadata unless the task says so.

## Build and check

```sh
cargo build --release
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI also enforces:

```sh
cargo clippy -p unko-nes --features apu-trace --all-targets -- -D warnings
cargo check -p unko-nes --no-default-features
cargo check -p unko-nes-host
cargo check -p unko-rom-manager --all-targets
cargo build --workspace --release
```

Dependency-boundary rules that CI checks:

- `unko-nes` must not depend on `unko-core`.
- `unko-rom-manager` must not depend on `unko-core` or `unko-nes`.

For a narrow change, run the focused crate first, then the workspace gate when it is practical:

```sh
cargo test -p <crate>
cargo clippy -p <crate> --all-targets -- -D warnings
cargo fmt --all --check
```

## Platform notes

- Linux desktop builds need the system libraries listed in `.github/workflows/ci.yml` (ALSA, X11/Xkb/Wayland, udev, GBm/DRM/EGL, PulseAudio, ninja). If the GUI crate cannot build because a system library is missing, say so; do not claim the workspace passed.
- `unko-nes` is standalone and must build without the host crates.
- Test ROMs are not in the repository. A test that reports "skipped" because `test-roms/` is missing is not coverage.
- `cargo test --workspace` is the gate; a single passing crate is not the gate.

## Agent rules

- Work on a feature branch or in a separate worktree. Never commit directly to `main` or `master`.
- One logical change per branch. Do not mix a fix with a refactor, a dependency bump, or a docs rewrite.
- Read the surrounding code, tests, and conventions before editing. Match naming, formatting, and error handling.
- Make the smallest change that satisfies the task. Do not reformat unrelated files or edit generated/lock files.
- Add or update tests for changed behaviour. Every bug fix gets a regression test that fails before the fix.
- Do not add dependencies without an explicit instruction. If a dependency seems necessary, stop and report.
- Do not disable or weaken a check to make a test pass. Do not delete a failing test.
- Report commands actually run and their real results. "Not run", "skipped", and "blocked" are valid; "all tests pass" is only valid when they did.

## Accuracy work

Accuracy changes are higher risk than ordinary code:

1. Pick a failure from the relevant ledger (`docs/accuracy-divergences.md`, `docs/snes-accuracy-divergences.md`).
2. Reproduce it with the documented runner before changing anything.
3. Make the smallest ares-faithful change. Do not patch to the answer key.
4. Re-run the isolated case, the nearby cases, and the full battery.
5. Update `docs/accuracy-status.md` and the ledger with a dated entry and the evidence.

Never edit an accuracy ledger to claim a fix that a test did not demonstrate.

## Handoff

Report: what changed, files touched, commands run with results, what was not verified, and any decision the reviewer must make. Keep the diff and the report consistent; if they disagree, the diff is the truth.
