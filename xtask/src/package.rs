//! Deterministic release archives and checksums.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ASSETS: &str = "assets";

fn workspace_version() -> Result<String, String> {
    let text = fs::read_to_string("Cargo.toml").map_err(|err| format!("Cargo.toml: {err}"))?;
    let value: toml::Value = toml::from_str(&text).map_err(|err| format!("Cargo.toml: {err}"))?;
    value["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "workspace.package.version is missing".into())
}

fn source_epoch() -> u64 {
    if let Ok(value) = std::env::var("SOURCE_DATE_EPOCH") {
        if let Ok(epoch) = value.parse() {
            return epoch;
        }
    }
    Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn copy_file(source: &Path, destination: &Path, mode: u32) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!("missing file: {}", source.display()));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    fs::copy(source, destination).map_err(|err| format!("copy {}: {err}", source.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(mode))
            .map_err(|err| format!("chmod {}: {err}", destination.display()))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(source)
        .map_err(|err| format!("{}: {err}", source.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &destination.join(entry.file_name()))?;
        } else {
            copy_file(&path, &destination.join(entry.file_name()), 0o644)?;
        }
    }
    Ok(())
}

fn stage(
    platform: &str,
    arch: &str,
    target_dir: &Path,
    staging: &Path,
    version: &str,
) -> Result<PathBuf, String> {
    let stem = format!("unko-{platform}-{arch}");
    let root = staging.join(&stem);
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|err| format!("{}: {err}", root.display()))?;
    }
    fs::create_dir_all(&root).map_err(|err| format!("{}: {err}", root.display()))?;
    let exe = if platform == "windows" { ".exe" } else { "" };
    if platform == "macos" {
        let app = root.join("Unko.app/Contents");
        copy_file(
            &target_dir.join("release/unko-app"),
            &app.join("MacOS/unko-app"),
            0o755,
        )?;
        fs::write(app.join("Info.plist"), info_plist(version))
            .map_err(|err| format!("Info.plist: {err}"))?;
        let icon = Path::new(ASSETS).join("unko.icns");
        if icon.is_file() {
            copy_file(&icon, &app.join("Resources/unko.icns"), 0o644)?;
        }
        copy_file(&target_dir.join("release/unko"), &root.join("unko"), 0o755)?;
    } else {
        copy_file(
            &target_dir.join(format!("release/unko{exe}")),
            &root.join(format!("unko{exe}")),
            0o755,
        )?;
        copy_file(
            &target_dir.join(format!("release/unko-app{exe}")),
            &root.join(format!("unko-app{exe}")),
            0o755,
        )?;
    }
    if platform == "linux" {
        copy_file(
            &Path::new(ASSETS).join("unko.desktop"),
            &root.join("share/applications/unko.desktop"),
            0o644,
        )?;
        copy_file(
            &Path::new(ASSETS).join("unko-256.png"),
            &root.join("share/icons/hicolor/256x256/apps/unko.png"),
            0o644,
        )?;
    }
    copy_file(Path::new("LICENSE"), &root.join("LICENSE"), 0o644)?;
    copy_file(Path::new("NOTICE"), &root.join("NOTICE"), 0o644)?;
    copy_file(Path::new("README.md"), &root.join("README.md"), 0o644)?;
    copy_tree(Path::new("licenses"), &root.join("licenses"))?;
    Ok(root)
}

fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>Unko</string>
    <key>CFBundleExecutable</key><string>unko-app</string>
    <key>CFBundleIconFile</key><string>unko.icns</string>
    <key>CFBundleIdentifier</key><string>dev.unko.app</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>Unko</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    )
}

fn files_under(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    fn walk(root: &Path, current: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<(), String> {
        let mut entries: Vec<_> = fs::read_dir(current)
            .map_err(|err| format!("{}: {err}", current.display()))?
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else {
                let name = path
                    .strip_prefix(root)
                    .map_err(|err| err.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((path, name));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

fn tar_gz(root: &Path, output: &Path, epoch: u64) -> Result<(), String> {
    let file = File::create(output).map_err(|err| format!("{}: {err}", output.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    let prefix = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unko");
    for (path, name) in files_under(root)? {
        let name = format!("{prefix}/{name}");
        let mut data = File::open(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let mut header = Header::new_ustar();
        let size = data.metadata().map_err(|err| err.to_string())?.len();
        header.set_size(size);
        header.set_mode(if is_executable(&path) { 0o755 } else { 0o644 });
        header.set_mtime(epoch);
        header.set_uid(0);
        header.set_gid(0);
        header.set_username("").map_err(|err| err.to_string())?;
        header.set_groupname("").map_err(|err| err.to_string())?;
        builder
            .append_data(&mut header, name, &mut data)
            .map_err(|err| err.to_string())?;
    }
    let encoder = builder.into_inner().map_err(|err| err.to_string())?;
    encoder.finish().map_err(|err| err.to_string())?;
    Ok(())
}

fn zip_datetime(epoch: u64) -> zip::DateTime {
    let days = (epoch / 86_400) as i64;
    let seconds = epoch % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = (seconds / 3600) as u8;
    let minute = ((seconds % 3600) / 60) as u8;
    let second = (seconds % 60) as u8;
    let year = year.clamp(1980, 2107) as u16;
    zip::DateTime::from_date_and_time(year, month as u8, day as u8, hour, minute, second)
        .unwrap_or_default()
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    ((y + if m <= 2 { 1 } else { 0 }) as i32, m as u32, d as u32)
}

fn zip_archive(root: &Path, output: &Path, epoch: u64) -> Result<(), String> {
    let file = File::create(output).map_err(|err| format!("{}: {err}", output.display()))?;
    let mut writer = ZipWriter::new(file);
    let stamp = zip_datetime(epoch);
    let prefix = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unko");
    for (path, name) in files_under(root)? {
        let name = format!("{prefix}/{name}");
        let mut data = Vec::new();
        File::open(&path)
            .map_err(|err| format!("{}: {err}", path.display()))?
            .read_to_end(&mut data)
            .map_err(|err| err.to_string())?;
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(stamp)
            .unix_permissions(if is_executable(&path) { 0o755 } else { 0o644 });
        writer
            .start_file(name, options)
            .map_err(|err| err.to_string())?;
        writer.write_all(&data).map_err(|err| err.to_string())?;
    }
    writer.finish().map_err(|err| err.to_string())?;
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    }
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut file = File::open(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mut platform = None;
    let mut arch = std::env::consts::ARCH.to_string();
    let mut out = PathBuf::from("dist");
    let mut target_dir = PathBuf::from("target");
    let mut version = None;
    let mut index = 0;
    while index < args.len() {
        let value = args.get(index + 1);
        match args[index].as_str() {
            "--platform" => {
                platform = value.cloned();
                index += 1;
            }
            "--arch" => {
                arch = value.cloned().ok_or("--arch needs a value")?;
                index += 1;
            }
            "--out" => {
                out = PathBuf::from(value.ok_or("--out needs a path")?);
                index += 1;
            }
            "--target-dir" => {
                target_dir = PathBuf::from(value.ok_or("--target-dir needs a path")?);
                index += 1;
            }
            "--version" => {
                version = value.cloned();
                index += 1;
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    let platform = platform.ok_or("--platform is required")?;
    let version = version.map_or_else(workspace_version, Ok)?;
    let epoch = source_epoch();
    fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
    let staging = std::env::temp_dir().join(format!(
        "unko-package-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));
    fs::create_dir_all(&staging).map_err(|err| format!("{}: {err}", staging.display()))?;
    let result = (|| {
        let root = stage(&platform, &arch, &target_dir, &staging, &version)?;
        let stem = format!("unko-{platform}-{arch}");
        let output = match platform.as_str() {
            "windows" => {
                let path = out.join(format!("{stem}.zip"));
                zip_archive(&root, &path, epoch)?;
                path
            }
            "linux" | "macos" => {
                let path = out.join(format!("{stem}.tar.gz"));
                tar_gz(&root, &path, epoch)?;
                path
            }
            other => return Err(format!("unknown platform {other}")),
        };
        let digest = sha256(&output)?;
        let mut entries = Vec::new();
        let mut files: Vec<_> = fs::read_dir(&out)
            .map_err(|err| format!("{}: {err}", out.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file() && path.file_name().is_some_and(|name| name != "SHA256SUMS")
            })
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?");
            entries.push(format!("{}  {name}", sha256(&path)?));
        }
        fs::write(out.join("SHA256SUMS"), entries.join("\n") + "\n")
            .map_err(|err| format!("SHA256SUMS: {err}"))?;
        println!(
            "{}: {} bytes, sha256 {digest}",
            output
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?"),
            fs::metadata(&output).map_or(0, |meta| meta.len())
        );
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}
