# Non-Rust code in the repository

The emulator, CLI, desktop shell, netplay, ROM manager, tests and repository
tooling are Rust. This is the complete first-party non-Rust inventory as of
2026-11-16.

## Platform source code

| File | Language | Lines | Why it is not Rust |
|---|---|---:|---|
| `crates/unko-app/ios/Sources/main.m` | Objective-C | 12 | iOS `main` must call the Rust `start_winit_app` from the app's main thread; UIKit owns the process entry. |
| `crates/unko-app/android/build.gradle.kts` | Kotlin DSL | 3 | Android Gradle plugin build script. |
| `crates/unko-app/android/settings.gradle.kts` | Kotlin DSL | 17 | Gradle module/include configuration. |
| `crates/unko-app/android/app/build.gradle.kts` | Kotlin DSL | 28 | Android application/NDK packaging configuration. |

The Android entry point itself is Rust (`android_main` in
`crates/unko-app/src/lib.rs`); Gradle is only the packaging layer. A Rust
`main` for iOS would still need the Objective-C/UIKit runtime, so the 12-line
trampoline is the smallest platform shim unless we adopt `objc2`-based process
setup.

## Build and maintenance tooling

All former Python and shell tooling is now the Rust **`xtask`** crate:

```sh
cargo run -p xtask -- license-audit
cargo run -p xtask -- osv-audit
cargo run -p xtask -- snes-scan --dir roms/snes --frames 300 --batch 5
cargo run -p xtask -- repair-vs <dir> -o test-roms/vs
cargo run -p xtask -- package --platform linux --arch x86_64
cargo run -p xtask -- pgo <rom-dir>
cargo run -p xtask -- appimage dist
cargo run -p xtask -- icns
cargo run -p xtask -- android-build
cargo run -p xtask -- ios-build
```

Those commands still invoke platform SDKs/tools (Xcode, Gradle, `cargo-ndk`,
`linuxdeploy`, AppImageTool, LLVM `llvm-profdata`), but the orchestration is
Rust. `xtask/osv-baseline.txt` is reviewed data, not code.

## Configuration and data (not application code)

| File | Format |
|---|---|
| `.github/workflows/ci.yml`, `release.yml` | GitHub Actions YAML |
| `crates/unko-app/android/app/src/main/AndroidManifest.xml` | Android XML |
| `crates/unko-app/android/gradle.properties` | Java properties |
| `crates/unko-app/ios/project.yml` | XcodeGen YAML |
| `crates/unko-app/ios/Sources/Info.plist` | Apple plist |
| `assets/unko.desktop` | freedesktop entry |
| `Cargo.toml`, `Cargo.lock` | Cargo configuration |
| `README.md`, `NOTICE`, `LICENSE`, `docs/**` | documentation/licensing |

## Third-party code

- **SDL3** is compiled from the `sdl3-src` crate (C) for desktop input/audio.
  It is not vendored in this repository and is not first-party code.
- `unko-nes` includes algorithms adapted from **ares** (ISC), reimplemented in
  Rust; the original C++ is not included.
- No first-party C, C++, JavaScript, TypeScript, GLSL or WGSL files are
  tracked. Shader sources are Rust string literals in
  `crates/unko-app/src/gpu_shader.rs` and `crates/unko-app/src/vulkan.rs`.

## Suggested review order

1. Keep `main.m` and the three Gradle files as platform-required shims.
2. Keep workflow YAML and platform manifests as configuration.
3. Review `xtask` for any additional repository automation; no Python/shell
   code remains.
