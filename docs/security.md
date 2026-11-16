# Security audit

Date: 2026-11-16
Commit: `4421f00` (plus the README/docs pass in this change)

Scope: the whole repository, its committed dependency lockfile, GitHub
workflows, scripts and the paths that consume untrusted input (ROMs, archives,
save states, netplay datagrams, relay rooms and DAT downloads).

## Method

- Queried OSV.dev for every crates.io package in `Cargo.lock` (510 packages).
- Ran the repository license allowlist check (`scripts/license_audit.py`).
- Scanned tracked files and full git history for credentials, private keys and
  common token formats; checked for sensitive file names and world-writable
  files.
- Reviewed the `unsafe` surface, panic/error handling in untrusted parsers,
  archive/network bounds, release workflow permissions and action pinning.
- Re-ran the functional gates (clippy, fmt, tests, AccuracyCoin, mapper suite,
  synthetic benchmark).

## Findings

### 1. High advisory: quick-xml 0.30.0 (transitive, desktop accessibility)

`RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` (CVSS 3.1 7.5, availability only)
affect quick-xml before 0.41.0: quadratic duplicate-attribute checking and
unbounded namespace allocation during XML parsing.

Path: `quick-xml 0.30.0 -> zbus_xml -> zbus-lockstep -> atspi -> accesskit_unix
-> egui-winit -> eframe -> unko-app`. The affected parser processes local
D-Bus accessibility introspection XML, not ROM/network input, so practical
exposure is limited to a malicious local D-Bus service or accessibility tree.
`unko-rom-manager` already uses the fixed quick-xml 0.41 line.

Recommendation: upgrade eframe/egui/accesskit to a release that uses
quick-xml >= 0.41, or build the desktop target without the `accesskit` feature
until then (losing screen-reader integration). Track as a dependency-upgrade
item, not an emulator-core issue.

### 2. Unmaintained crates (no known CVE)

- `bincode 1.3.3` — `RUSTSEC-2025-0141`. Used for save-state/mapper state and
  GGRS wire messages. The format is versioned by `UNKOST21`; migration to
  bincode 2 or another format needs a save-state compatibility plan.
- `instant 0.1.13` — `RUSTSEC-2024-0384`, pulled by GGRS.
- `paste 1.0.15` — `RUSTSEC-2024-0436`.
- `ttf-parser 0.25.1` — `RUSTSEC-2026-0192`, pulled by egui text shaping.
- `zbus_names`, `zvariant`, `zvariant_derive` 3/4 — `RUSTSEC-2026-0370/0371/0372`,
  pulled by the desktop accessibility stack.

No published memory-safety or code-execution advisory currently applies to
these versions. GGRS/netplay input is deserialized defensively; malformed
datagrams return errors rather than panicking.

### 3. Archive decompression has no size cap (medium/low, local)

`unko-rom::load_zip` trusts `member.size()` for `Vec::with_capacity` and then
calls `read_to_end`; 7z/RAR loading trusts archive metadata as well. A
malicious local archive could force a large allocation/decompression
(compression bomb) even though it never extracts paths to disk. ZIP/7z/RAR
members are streamed into memory only; no zip-slip path write was found.

Recommendation: add a 64 MiB ROM cap checked against `RomEntry.size` and the
ZIP member length, use `take(cap + 1)` for the actual read, and reject entries
whose declared/actual size exceeds it.

### 4. Netplay is plaintext and unauthenticated (medium over Internet)

GGRS runs over UDP with no transport encryption or peer authentication. The
relay room token is a deterministic FNV-1a hash of the room name, not a secret,
so anyone who knows the relay endpoint and room can join and inject datagrams.
This is acceptable for trusted LAN/friends play but should be documented as a
trust boundary. Recommendation: optional pre-shared-key authentication/AEAD at
the transport layer, or a secret room token; do not describe it as private
against a hostile network.

### 5. Release supply chain (low/medium)

GitHub Actions are referenced by mutable tags (`actions/checkout@v5`,
`dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`,
`softprops/action-gh-release@v3`, etc.). The release job has `contents: write`
so `softprops/action-gh-release` can publish assets. A compromised action tag or
upstream account could affect release artifacts. CI itself is least-privilege
(`contents: read`).

Recommendation: pin every action to a commit SHA, keep per-job permissions
minimal, and add an environment / manual approval for the publish job. Add a
scheduled `cargo audit`/OSV check so new advisories are visible in CI.

### 6. Positive controls

- `unsafe` is forbidden in `unko-core`, `unko-rom`, `unko-rom-manager`,
  `unko-nes`, `unko-nes-api`, `unko-nes-host`, `unko-netplay`, `unko-cli` and
  the app binary. The only exceptions are the OpenGL shader module and the
  mobile exported entry points, both explicitly scoped with `allow`.
- No credentials, private keys or token patterns in tracked files or full git
  history; no `.env`, key stores or sensitive filenames tracked; no
  world-writable tracked files.
- Dependency licenses: **510 packages, all allowed**.
- DAT download is bounded at 256 MiB, validated as Logiqx XML, written to a
  temporary file and atomically renamed.
- ROM manager rebuild names are reduced to `file_name` and sanitized; it does
  not extract archive paths.
- Save states check magic/version, ROM fingerprint and mapper state lengths;
  malformed states return errors.

## Gate results from this audit

- `cargo test --workspace`: 695 passed, 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all -- --check`: clean
- `nestest`: 8,980 / 8,980 lines exact
- AccuracyCoin: 146/151, 0 failed, 5 DRAW skips, 0 unrun
- Mapper suite: 65/65 cases, 0 failed
- Synthetic benchmark: 676.7 fps, 11.26x realtime (CI floor 300 fps)
- License audit: 510 dependencies, all allowed
- OSV: 9 advisories — 2 high (quick-xml DoS, transitive), 7 unmaintained

## Recommended next actions

1. Upgrade the desktop accessibility stack (or disable `accesskit`) to remove
   quick-xml 0.30.
2. Add archive size caps in `unko-rom`.
3. Add netplay authentication/encryption or explicitly document trusted-LAN
   use and secret rooms.
4. Pin GitHub Actions to commit SHAs and add a scheduled dependency-advisory
   job.
5. Plan the bincode 1.x migration together with the next save-state magic
   bump.

No ROMs, BIOS images or secrets are part of this repository.
