# Security policy

Unko is an emulator shell. The highest-risk surfaces are ROM/archive parsing,
save-state loading, netplay datagrams and the relay, and the DAT downloader.

## Reporting a vulnerability

Please use GitHub's **private vulnerability reporting** for this repository
(Security → Report a vulnerability) instead of opening a public issue. Include
the affected crate, a minimal reproducer where possible, and whether the issue
requires a malicious local file, a malicious netplay peer, or a malicious
relay/room.

Do not include copyrighted ROMs, BIOS images or personal data in a report.

## Supported versions

The `main` branch is the supported development line. Release artifacts are
built from tagged commits; older tags may not receive fixes.

## Scope

In scope:

- memory-safety, denial-of-service and path-handling bugs in the repository;
- save-state/ROM/archive parser crashes or unbounded allocations;
- netplay/relay authentication or injection issues;
- CI/release supply-chain issues.

Out of scope:

- emulation accuracy differences (use the accuracy issue tracker);
- ROMs, BIOS images or copyrighted firmware (never included here);
- vulnerabilities in third-party dependencies that are already covered by an
  upstream advisory, unless Unko's use makes them exploitable in a new way.

The current audit, findings and remediation notes are in
[`docs/security.md`](docs/security.md).
