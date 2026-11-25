//! Repository automation for Unko.
//!
//! This replaces the former Python and shell scripts so build, packaging,
//! licence, advisory and platform helpers are Rust too. Platform SDKs
//! (Xcode, Gradle, linuxdeploy, LLVM tools) are still invoked as external
//! tools.

mod license;
mod osv;
mod package;
mod platform;
mod repair;
mod snes_scan;

use std::process::ExitCode;

fn usage() -> &'static str {
    "usage:
  xtask snes-scan --dir ROMS | --list IMAGES [--ipl IPL.ROM] [--frames N]
  xtask license-audit
  xtask osv-audit
  xtask repair-vs <source-dir> [-o <output-dir>] [--all]
  xtask package --platform linux|windows|macos --arch <arch> [--out DIR]
                [--target-dir DIR] [--version VERSION]
  xtask pgo <rom-dir> [output-target-dir]
  xtask appimage [out-dir]
  xtask icns
  xtask android-build
  xtask ios-build"
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    };
    let rest = &args[1..];
    let result = match command {
        "snes-scan" => match snes_scan::run(rest) {
            0 => Ok(()),
            code => Err(format!("snes-scan exited with status {code}")),
        },
        "license-audit" => license::run(rest),
        "osv-audit" => osv::run(rest),
        "repair-vs" => repair::run(rest),
        "package" => package::run(rest),
        "pgo" => platform::pgo(rest),
        "appimage" => platform::appimage(rest),
        "icns" => platform::icns(rest),
        "android-build" => platform::android_build(rest),
        "ios-build" => platform::ios_build(rest),
        "-h" | "--help" | "help" => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown command {other}\n{}", usage())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
