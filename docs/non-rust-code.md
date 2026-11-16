# Non-Rust code in the repository

The emulator, CLI, desktop shell, netplay, ROM manager and tests are Rust.
This is the complete first-party non-Rust inventory as of 2026-11-16.

## Platform source code

| File | Language | Lines | Why it is not Rust |
|---|---|---:|---|
| `crates/unko-app/ios/Sources/main.m` | Objective-C | 12 | iOS `main` must call the Rust `start_winit_app` from the app's main thread; UIKit owns the process entry. |
| `crates/unko-app/android/build.gradle.kts` | Kotlin DSL | 3 | Android Gradle plugin build script. |
| `crates/unko-app/android/settings.gradle.kts` | Kotlin DSL | 17 | Gradle module/include configuration. |
| `crates/unko-app/android/app/build.gradle.kts` | Kotlin DSL | 28 | Android application/NDK packaging configuration. |

The Android entry point itself is Rust (`android_main` in `crates/unko-app/src/lib.rs`);
Gradle is only the packaging layer. A Rust `main` for iOS would still need the
Objective-C/UIKit runtime, so the 12-line trampoline is the smallest platform
shim unless we adopt `objc2`-based process setup.

## Build and maintenance tooling

| File | Language | Lines | Purpose |
|---|---|---:|---|
| `scripts/package.py` | Python | 225 | Deterministic release archives and checksums. |
| `scripts/license_audit.py` | Python | 145 | Dependency license allowlist. |
| `scripts/repair_vs_headers.py` | Python | 139 | Local VS. header repair/staging. |
| `scripts/osv_audit.py` | Python | 81 | OSV advisory gate. |
| `scripts/appimage.sh` | Bash | 64 | AppImage build. |
| `scripts/pgo.sh` | Bash | 39 | Profile-guided build. |
| `scripts/make_icns.sh` | Bash | 17 | macOS icon generation. |
| `crates/unko-app/android/build.sh` | Bash | 21 | `cargo-ndk` + Gradle build. |
| `crates/unko-app/ios/build.sh` | Bash | 23 | XcodeGen + Xcode build. |

These are candidates for a Rust `xtask` binary if the “all code in Rust” rule
includes build scripts. The AppImage and Xcode/Gradle steps still invoke
external tools; Rust would orchestrate them rather than replace them.

## Configuration and data (not application code)

| File | Format |
|---|---|
| `.github/workflows/ci.yml`, `release.yml` | GitHub Actions YAML |
| `crates/unko-app/android/app/src/main/AndroidManifest.xml` | Android XML |
| `crates/unko-app/android/gradle.properties` | Java properties |
| `crates/unko-app/ios/project.yml` | XcodeGen YAML |
| `crates/unko-app/ios/Sources/Info.plist` | Apple plist |
| `assets/unko.desktop` | freedesktop entry |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain` (if present) | Cargo/toolchain config |
| `README.md`, `NOTICE`, `LICENSE`, `docs/**` | documentation/licensing |

## Third-party code

- **SDL3** is compiled from the `sdl3-src` crate (C) for desktop input/audio.
  It is not vendored in this repository and is not first-party code.
- `unko-nes` includes algorithms adapted from **ares** (ISC), reimplemented in
  Rust; the original C++ is not included.
- No first-party C, C++, JavaScript, TypeScript, GLSL or WGSL files are tracked.
  Shader sources are Rust string literals in `crates/unko-app/src/gpu_shader.rs`
  and `crates/unko-app/src/vulkan.rs`.

## Suggested review order

1. Leave `main.m` and the three Gradle files as platform-required shims.
2. If build scripts must be Rust, port the five shell scripts first, then the
   four Python tools into one `xtask` crate.
3. Keep workflow YAML and platform manifests as configuration; porting them to
   Rust would not remove the underlying platform formats.
