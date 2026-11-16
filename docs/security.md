# Security audit

Date: 2026-11-16 (post-fix pass)

Scope: the whole repository, its committed dependency lockfile, GitHub
workflows, scripts and the paths that consume untrusted input (ROMs, archives,
save states, netplay datagrams, relay rooms and DAT downloads).

## Method

- Queried OSV.dev for every crates.io package in `Cargo.lock` (469 after adding the
  SNES CPU oracle and Rust `xtask` crate).
- Ran the repository license allowlist check (`xtask license-audit`).
- Scanned tracked files and full git history for credentials, private keys and
  common token formats; checked sensitive file names and world-writable files.
- Reviewed the `unsafe` surface, panic/error handling in untrusted parsers,
  archive/network bounds, release workflow permissions and action pinning.
- Re-ran the functional gates (clippy, fmt, tests, AccuracyCoin, mapper suite,
  synthetic benchmark).

## Fixes applied

### 1. High advisory: quick-xml 0.30 (fixed)

`RUSTSEC-2026-0194`/`0195` (CVSS 3.1 7.5, availability only) affected
quick-xml before 0.41 via `zbus_xml -> atspi -> accesskit -> egui-winit ->
eframe`. The desktop `accesskit` feature is now disabled, which removes the
whole `zbus`/quick-xml 0.30 chain. The tree now contains only quick-xml 0.41
(used by `unko-rom-manager`). Screen-reader integration is deferred until the
desktop stack upgrades; this is documented in `crates/unko-app/Cargo.toml`.

Result: **0 high advisories**; OSV now reports only the four unmaintained
crates below.

### 2. Archive size cap (fixed)

`unko-rom` now enforces `MAX_ROM_BYTES = 64 MiB`:

- `load` rejects an entry whose declared size exceeds the cap before any I/O;
- ZIP members are checked against their declared size, allocated with a
  bounded hint and read through `take(cap + 1)`, then the actual length is
  checked again.
- 7z members are streamed through `for_each_entries` with a bounded `take`;
  RAR parsing/extraction uses `max_member_output_bytes` and
  `max_total_output_bytes` set to the same cap.

A unit test covers the pre-I/O rejection. ZIP/7z/RAR members are still read
into memory only; no archive path is ever extracted to disk, so zip-slip is
not applicable.

### 3. CI supply chain (fixed)

- Every GitHub Action in `.github/workflows/ci.yml` and `release.yml` is now
  pinned to a commit SHA, with the version in a trailing comment.
- A new CI job runs `xtask osv-audit`; it queries OSV.dev for every
  crates.io package in `Cargo.lock` and fails when an advisory ID is not in
  `xtask/osv-baseline.txt`.
- CI remains `contents: read`; the release workflow keeps `contents: write`
  only for publishing.

### 4. Netplay trust boundary (accepted, documented)

GGRS runs over plaintext UDP with no peer authentication. The relay room token
is a deterministic FNV-1a hash of the room name, not a secret, so anyone who
knows the relay endpoint and room can join or inject datagrams. The NetPlay
window now displays a warning to use trusted peers or a private relay, and
`docs/netplay.md` documents the boundary. Optional pre-shared-key
authentication/AEAD remains a recommended follow-up; it is not implemented in
this pass.

## Remaining findings

### Unmaintained crates (no known CVE)

- `bincode 1.3.3` — `RUSTSEC-2025-0141`. Used for save-state/mapper state and
  GGRS wire messages; migration needs a save-state compatibility plan.
- `instant 0.1.13` — `RUSTSEC-2024-0384`, pulled by GGRS.
- `paste 1.0.15` — `RUSTSEC-2024-0436`.
- `ttf-parser 0.25.1` — `RUSTSEC-2026-0192`, pulled by egui text shaping.

These are accepted in `xtask/osv-baseline.txt`; a new advisory ID fails CI.
Malformed GGRS datagrams are handled as errors rather than panics, but the
plaintext/unauthenticated transport remains an Internet-facing trust boundary.

## Positive controls

- `unsafe` is forbidden in `unko-core`, `unko-rom`, `unko-rom-manager`,
  `unko-nes`, `unko-nes-api`, `unko-nes-host`, `unko-netplay`, `unko-cli` and
  the app binary. Only the OpenGL shader module and mobile exported entry
  points are explicitly allowed.
- No credentials, private keys or token patterns in tracked files or full git
  history; no `.env`, key stores or sensitive filenames tracked; no
  world-writable tracked files.
- Dependency licenses: all allowed (allowlist audit).
- DAT download is bounded at 256 MiB, validated as Logiqx XML, written to a
  temporary file and atomically renamed.
- Relay rooms/members, per-client relay queues, hole-punch queues and GGRS
  socket polls are bounded, dropping excess UDP datagrams instead of growing
  memory without limit.
- ROM manager rebuild names are reduced to `file_name` and sanitized; it does
  not extract archive paths.
- Save states check magic/version, ROM fingerprint and mapper state lengths;
  malformed states return errors.

## Gate results from this pass

- `cargo test --workspace`: 772 passed, 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all -- --check`: clean
- `nestest`: 8,980 / 8,980 lines exact
- AccuracyCoin: 146/151, 0 failed, 5 DRAW skips, 0 unrun
- Mapper suite: 65/65 cases, 0 failed
- Synthetic benchmark: 780.5 fps, 12.99x realtime (CI floor 300 fps)
- License audit: 466 dependencies, all allowed
- OSV: 469 packages, 4 advisories (all unmaintained, baseline), 0 new
- `cargo metadata --locked`: clean

## Recommended next actions

1. Add optional netplay authentication/encryption (PSK + AEAD) or keep the
   trusted-LAN warning prominent.
2. Plan the bincode 1.x migration with the next save-state magic bump.
3. Upgrade egui/eframe when convenient to move `ttf-parser` forward and
   restore accessibility with a fixed quick-xml.
4. Keep `xtask/osv-baseline.txt` reviewed on every dependency upgrade.

No ROMs, BIOS images or secrets are part of this repository.
