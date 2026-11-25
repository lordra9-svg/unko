# Security audit

Last full audit: **2026-11-25**. Scope: the whole repository, `Cargo.lock`,
GitHub workflows, scripts, licenses, and the paths that consume untrusted
input (ROMs, archives, save states, netplay datagrams, relay rooms, DAT
downloads, firmware paths).

The policy and reporting process are in [`SECURITY.md`](../SECURITY.md).

## Method

- `cargo audit` against the current lockfile (487 crates).
- `cargo run -p xtask -- license-audit` for the dependency license allowlist.
- Manual review of every `unsafe` block, every path that writes files, all
  archive/network code and the save-state decoders.
- Review of the CI/release workflows (action pinning, token permissions) and
  the DAT downloader.
- Full test/clippy/fmt verification after the fixes (see **Verification**).

## 2026-11-25 findings and fixes

### 1. `url` stack overflow (fixed)

`RUSTSEC-2026-0592` (CVSS 7.5 availability): `url 2.5.8` can stack-overflow in
`Url::origin()` on nested `blob:` URLs. It entered through `reqwest` and
`webbrowser` (egui). Unko never calls `origin()` on attacker URLs, so the path
was not reachable, but the dependency was updated:

```
url 2.5.8 -> 2.5.9
form_urlencoded 1.2.2 -> 1.2.3
```

`cargo audit` now reports **0 vulnerabilities**.

### 2. Unbounded UNIF read while scanning ZIPs (fixed)

The ZIP scanner read only a 16-byte prefix for iNES/FDS members, but read the
whole member for `.unif`/`.unf` so UNIF chunks could be parsed. That path had
no decompression bound. It now uses `take(MAX_ROM_BYTES + 1)` like the load
path, so scanning a malicious archive cannot inflate past 64 MiB.

### 3. Unbounded save-state decoding (fixed)

NES and SNES states are `bincode` blobs. `bincode::deserialize` trusts encoded
lengths, so a small malicious state could request a huge allocation.

- `unko-nes`: `load_state` rejects blobs over 64 MiB before decoding; the
  system and mapper decoders use
  `DefaultOptions::new().with_fixint_encoding().allow_trailing_bytes()
  .with_limit(64 MiB)` (same wire format as before).
- `unko-snes`: `load_state` rejects blobs over 64 MiB and uses the same
  bounded, fixint-compatible options.
- Existing short-header/size/version checks remain. Save states are still
  documented as untrusted local input.

### 4. Temporary paths (accepted, low risk)

The desktop app creates scan/temporary directories named with the PID under
`std::env::temp_dir()`. These are not privileged and are created for local
scans/tests only; there is no world-writable shared location or setuid use. A
hostile local process with the same UID can already read the user's files.
`CPU_TRACE_LOG`/`CPU_TRACE_FROM`/`CPU_TRACE_TO` are opt-in debugging variables
that append a PC trace to a user-chosen path; they do not affect emulation.

### 5. Archive extraction and zip-slip (no issue)

Archives are read into memory to obtain the selected member; no archive path is
ever extracted to disk, so zip-slip is not applicable. ROM-manager rebuild
outputs are created from sanitized file names (path separators and reserved
characters replaced, trailing dots/spaces removed) and duplicate entries are
rejected. Loading enforces `MAX_ROM_BYTES = 64 MiB` for ZIP (declared and
actual), 7z (streamed `take`) and RAR (reader output caps).

### 6. DAT downloader (bounded, user-initiated)

`fetch_dat` accepts a user-supplied URL over `reqwest` with rustls native
roots, a 120 s timeout and a 256 MiB body cap. The body must parse as non-empty
Logiqx XML before it atomically replaces the destination (`write .part`, then
`rename`). This is a local CLI action; there is no remote trigger and no
credential handling.

### 7. Netplay / relay trust boundary (accepted, documented)

GGRS runs over plaintext UDP with no peer authentication. The relay room token
is a deterministic hash of the room name, not a secret: anyone who knows the
endpoint and room can join or inject datagrams. Relay resources are bounded
(datagram 2048 bytes, 256 queued per client, 64 rooms × 16 members). The UI
warns to use trusted peers or a private relay; see `docs/netplay.md`.
Pre-shared-key/AEAD authentication remains a recommended follow-up.

### 8. `unsafe` surface

Only `unko-app` is not `forbid(unsafe_code)`: `deny(unsafe_code)` with
allowlisted scopes for OpenGL/glow shader setup in `gpu_shader.rs` and the two
exported `#[unsafe(no_mangle)]` mobile entry points in `lib.rs`. Every other
first-party crate (`unko-core`, `unko-rom`, `unko-rom-manager`, `unko-nes*`,
`unko-snes*`, `unko-n64*`, `unko-netplay`, `unko-cli`, `xtask`) has
`#![forbid(unsafe_code)]`. Third-party `corosensei` (SNES coroutines) and the
GUI/graphics stack are not first-party code; their licenses are audited.

### 9. Supply chain / CI

- Every GitHub Action is pinned to a commit SHA with the version in a comment.
- CI is `contents: read`; only the release publish job has `contents: write`.
- The `osv-audit` job checks every crates.io package in `Cargo.lock` and fails
  on advisory IDs not listed in `xtask/osv-baseline.txt`.
- `license-audit`: **468 dependencies, all licenses allowed** (MIT target).

## Remaining accepted warnings

`cargo audit` reports four **unmaintained** crates with no known exploit and no
fixed upgrade:

| Crate | Advisory | Why it remains |
|---|---|---|
| `bincode 1.3.3` | RUSTSEC-2025-0141 | Save-state and GGRS wire format; migration needs a compatibility plan. Save-state decoding is size/depth bounded as above. |
| `instant 0.1.13` | RUSTSEC-2024-0384 | Pulled by GGRS. |
| `paste 1.0.15` | RUSTSEC-2024-0436 | Transitive macro dependency. |
| `ttf-parser 0.25.1` | RUSTSEC-2026-0192 | egui text shaping. |

None are reachable with attacker-controlled data in a way that produces memory
unsafety in first-party code; they are tracked for a future dependency refresh
(notably `bincode` → 2.x with state versioning).

## Verification (2026-11-25)

```
cargo audit                      -> 0 vulnerabilities, 4 allowed warnings
cargo run -p xtask -- license-audit -> 468 deps, all licenses allowed
cargo test --workspace           -> green (see README for counts)
cargo clippy --workspace --all-targets -> clean
cargo fmt --all -- --check       -> clean
```

SNES reference gates were re-run after the changes (600-frame audio, 0
differing samples for Mega Man X2 and Donkey Kong Country); see the commit log
for the exact commands and outputs.
