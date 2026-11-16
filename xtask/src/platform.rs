//! Platform helpers that orchestrate external SDK/tooling commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(mut command: Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|err| format!("{}: {err}", command.get_program().to_string_lossy()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{:?} exited with {status}", command))
    }
}

fn workspace_version() -> Result<String, String> {
    let text = fs::read_to_string("Cargo.toml").map_err(|err| format!("Cargo.toml: {err}"))?;
    let value: toml::Value = toml::from_str(&text).map_err(|err| format!("Cargo.toml: {err}"))?;
    value["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "workspace.package.version is missing".into())
}

pub fn pgo(args: &[String]) -> Result<(), String> {
    let rom_dir = args
        .first()
        .ok_or("usage: xtask pgo <rom-dir> [output-dir]")?;
    let out_dir = args
        .get(1)
        .map_or_else(|| PathBuf::from("target/pgo"), PathBuf::from);
    let temp = std::env::temp_dir().join(format!(
        "unko-pgo-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));
    fs::create_dir_all(&temp).map_err(|err| format!("{}: {err}", temp.display()))?;
    let gen_dir = PathBuf::from("target/pgo-gen");
    let result = (|| {
        let mut check = Command::new("llvm-profdata");
        check.arg("--version");
        if !check.status().is_ok_and(|status| status.success()) {
            return Err("llvm-profdata is required (install LLVM tools matching rustc)".into());
        }
        println!("==> instrumented build");
        let mut build = Command::new("cargo");
        build
            .args(["build", "--release", "--target-dir"])
            .arg(&gen_dir)
            .args(["-p", "unko-cli", "-p", "unko-app"])
            .env(
                "RUSTFLAGS",
                format!("-Cprofile-generate={}", temp.display()),
            );
        run(build)?;
        println!("==> training on {rom_dir}");
        let mut train = Command::new(gen_dir.join("release/unko"));
        train.args([
            "corpus", rom_dir, "--frames", "20", "--limit", "600", "--jobs", "0",
        ]);
        run(train)?;
        println!("==> merging profile");
        let profiles: Vec<_> = fs::read_dir(&temp)
            .map_err(|err| format!("{}: {err}", temp.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "profraw"))
            .collect();
        if profiles.is_empty() {
            return Err("no .profraw files were generated".into());
        }
        let mut merge = Command::new("llvm-profdata");
        merge
            .arg("merge")
            .arg("-o")
            .arg(temp.join("merged.profdata"))
            .args(profiles);
        run(merge)?;
        println!("==> profile-guided build");
        let mut build = Command::new("cargo");
        build
            .args(["build", "--release", "--target-dir"])
            .arg(&out_dir)
            .args(["-p", "unko-cli", "-p", "unko-app"])
            .env(
                "RUSTFLAGS",
                format!(
                    "-Cprofile-use={} -Cllvm-args=-pgo-warn-missing-function",
                    temp.join("merged.profdata").display()
                ),
            );
        run(build)?;
        println!("==> done");
        println!("CLI: {}", out_dir.join("release/unko").display());
        println!("App: {}", out_dir.join("release/unko-app").display());
        Ok(())
    })();
    let _ = fs::remove_dir_all(&temp);
    result
}

fn download(url: &str, destination: &Path) -> Result<(), String> {
    if destination.is_file() {
        return Ok(());
    }
    println!("fetching {}", destination.display());
    let response = reqwest::blocking::get(url).map_err(|err| format!("{url}: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let bytes = response.bytes().map_err(|err| format!("{url}: {err}"))?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    fs::write(destination, bytes).map_err(|err| format!("{}: {err}", destination.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("chmod {}: {err}", destination.display()))?;
    }
    Ok(())
}

pub fn appimage(args: &[String]) -> Result<(), String> {
    let out = args
        .first()
        .map_or_else(|| PathBuf::from("dist"), PathBuf::from);
    let tools = PathBuf::from("target/appimage-tools");
    let arch = std::env::var("ARCH").unwrap_or_else(|_| "x86_64".into());
    fs::create_dir_all(&tools).map_err(|err| format!("{}: {err}", tools.display()))?;
    fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
    let linuxdeploy = tools.join("linuxdeploy");
    let appimagetool = tools.join("appimagetool");
    download(
        &format!("https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-{arch}.AppImage"),
        &linuxdeploy,
    )?;
    download(
        &format!("https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-{arch}.AppImage"),
        &appimagetool,
    )?;

    let version = workspace_version()?;
    let mut build = Command::new("cargo");
    build.args([
        "build",
        "--release",
        "--locked",
        "-p",
        "unko-cli",
        "-p",
        "unko-app",
    ]);
    run(build)?;

    let appdir = PathBuf::from("target/AppDir");
    if appdir.exists() {
        fs::remove_dir_all(&appdir).map_err(|err| format!("{}: {err}", appdir.display()))?;
    }
    for (source, destination, mode) in [
        (
            Path::new("target/release/unko-app"),
            appdir.join("usr/bin/unko-app"),
            0o755,
        ),
        (
            Path::new("target/release/unko"),
            appdir.join("usr/bin/unko"),
            0o755,
        ),
        (
            Path::new("assets/unko-256.png"),
            appdir.join("usr/share/icons/hicolor/256x256/apps/unko.png"),
            0o644,
        ),
        (
            Path::new("LICENSE"),
            appdir.join("usr/share/doc/unko/LICENSE"),
            0o644,
        ),
        (
            Path::new("NOTICE"),
            appdir.join("usr/share/doc/unko/NOTICE"),
            0o644,
        ),
        (
            Path::new("assets/unko.desktop"),
            appdir.join("usr/share/applications/unko.desktop"),
            0o644,
        ),
        (
            Path::new("assets/unko.desktop"),
            appdir.join("unko.desktop"),
            0o644,
        ),
        (
            Path::new("assets/unko-256.png"),
            appdir.join("unko.png"),
            0o644,
        ),
    ] {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        fs::copy(source, &destination).map_err(|err| format!("{}: {err}", source.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(mode))
                .map_err(|err| format!("chmod {}: {err}", destination.display()))?;
        }
    }
    let sdl = fs::read_dir("target/release")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libSDL3.so"))
        });
    if let Some(sdl) = sdl {
        let name = sdl.file_name().unwrap_or_default();
        let destination = appdir.join("usr/lib").join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        fs::copy(&sdl, &destination).map_err(|err| format!("{}: {err}", sdl.display()))?;
    }
    let mut deploy = Command::new(&linuxdeploy);
    deploy
        .arg("--appdir")
        .arg(&appdir)
        .arg("--executable")
        .arg(appdir.join("usr/bin/unko-app"))
        .arg("--desktop-file")
        .arg(appdir.join("usr/share/applications/unko.desktop"))
        .arg("--icon-file")
        .arg(appdir.join("usr/share/icons/hicolor/256x256/apps/unko.png"))
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .env("LINUXDEPLOY_OUTPUT_VERSION", &version)
        .env("LD_LIBRARY_PATH", "target/release");
    run(deploy)?;
    let appdir = fs::canonicalize(&appdir).map_err(|err| format!("{}: {err}", appdir.display()))?;
    let out = fs::canonicalize(&out).map_err(|err| format!("{}: {err}", out.display()))?;
    let output = out.join(format!("unko-{version}-{arch}.AppImage"));
    let mut appimage = Command::new(&appimagetool);
    appimage
        .arg(&appdir)
        .arg(&output)
        .env("ARCH", &arch)
        .env("APPIMAGE_EXTRACT_AND_RUN", "1");
    run(appimage)?;
    println!("AppImage: {}", output.display());
    Ok(())
}

pub fn icns(_args: &[String]) -> Result<(), String> {
    let temp = std::env::temp_dir().join(format!("unko-icns-{}", std::process::id()));
    let iconset = temp.join("Unko.iconset");
    fs::create_dir_all(&iconset).map_err(|err| format!("{}: {err}", iconset.display()))?;
    let source = Path::new("assets/unko-512.png");
    for size in [16, 32, 64, 128, 256, 512] {
        for scale in [1, 2] {
            let pixels = size * scale;
            let name = if scale == 1 {
                format!("icon_{size}x{size}.png")
            } else {
                format!("icon_{size}x{size}@2x.png")
            };
            let mut command = Command::new("sips");
            command
                .args(["-z", &pixels.to_string(), &pixels.to_string()])
                .arg(source)
                .arg("--out")
                .arg(iconset.join(name));
            run(command)?;
        }
    }
    let mut command = Command::new("iconutil");
    command
        .args(["-c", "icns"])
        .arg(&iconset)
        .arg("-o")
        .arg("assets/unko.icns");
    let result = run(command);
    let _ = fs::remove_dir_all(&temp);
    result?;
    println!("wrote assets/unko.icns");
    Ok(())
}

pub fn android_build(_args: &[String]) -> Result<(), String> {
    let mut ndk = Command::new("cargo");
    ndk.args([
        "ndk",
        "-t",
        "arm64-v8a",
        "-o",
        "crates/unko-app/android/app/src/main/jniLibs",
        "build",
        "--release",
        "--lib",
        "-p",
        "unko-app",
    ]);
    run(ndk)?;
    let gradlew = std::fs::canonicalize("crates/unko-app/android/gradlew").ok();
    let mut gradle = if let Some(gradlew) = gradlew {
        Command::new(gradlew)
    } else {
        Command::new("gradle")
    };
    gradle.arg("assembleRelease");
    gradle.current_dir("crates/unko-app/android");
    run(gradle)?;
    println!("APK: crates/unko-app/android/app/build/outputs/apk/release/app-release-unsigned.apk");
    Ok(())
}

pub fn ios_build(_args: &[String]) -> Result<(), String> {
    let mut build = Command::new("cargo");
    build.args([
        "build",
        "--release",
        "--target",
        "aarch64-apple-ios",
        "--lib",
        "-p",
        "unko-app",
    ]);
    run(build)?;
    let mut generate = Command::new("xcodegen");
    generate
        .args(["generate", "--spec", "project.yml"])
        .current_dir("crates/unko-app/ios");
    run(generate)?;
    let mut xcode = Command::new("xcodebuild");
    xcode
        .args([
            "-project",
            "Unko.xcodeproj",
            "-scheme",
            "Unko",
            "-configuration",
            "Release",
            "-sdk",
            "iphoneos",
            "-derivedDataPath",
            "build",
            "build",
        ])
        .current_dir("crates/unko-app/ios");
    run(xcode)?;
    println!("App: crates/unko-app/ios/build/Build/Products/Release-iphoneos/Unko.app");
    Ok(())
}
