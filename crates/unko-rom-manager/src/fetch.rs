//! DAT download and update.
//!
//! A downloaded DAT is validated as Logiqx XML before it replaces the
//! destination, so a bad URL, an HTML error page or a truncated transfer can
//! never clobber a working catalog. The transfer is bounded and uses a
//! temporary file in the destination directory followed by a rename.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{parse_dat_str, Error, Result};

/// Largest accepted DAT transfer (256 MiB).
pub const MAX_DAT_BYTES: u64 = 256 * 1024 * 1024;

/// Downloads `url` and atomically replaces `destination` with it.
///
/// The body must parse as a Logiqx DAT; otherwise the destination is left
/// untouched.
///
/// # Errors
///
/// Returns [`Error::Http`] for transport/status failures, [`Error::Dat`] when
/// the body is not a DAT, and [`Error::Io`] when the file cannot be written.
pub fn fetch_dat(url: &str, destination: &Path) -> Result<PathBuf> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("unko-rom-manager/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| Error::Http(err.to_string()))?;
    let response = client
        .get(url)
        .send()
        .map_err(|err| Error::Http(format!("{url}: {err}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Http(format!("{url}: HTTP {status}")));
    }
    let mut body = Vec::new();
    response
        .take(MAX_DAT_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|err| Error::Http(format!("{url}: {err}")))?;
    if body.len() as u64 > MAX_DAT_BYTES {
        return Err(Error::Http(format!(
            "{url}: DAT is larger than {MAX_DAT_BYTES} bytes"
        )));
    }
    let text = String::from_utf8(body)
        .map_err(|_| Error::Dat(format!("{url}: DAT is not valid UTF-8")))?;
    let dat = parse_dat_str(&text)
        .map_err(|err| Error::Dat(format!("{url}: downloaded DAT is invalid: {err}")))?;
    if dat.games.is_empty() {
        return Err(Error::Dat(format!(
            "{url}: downloaded DAT contains no games (not a DAT?)"
        )));
    }

    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|err| Error::Io(format!("{}: {err}", parent.display())))?;
        }
    }
    let file_name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "dat.xml".into());
    let part = destination.with_file_name(format!(".{file_name}.part"));
    fs::write(&part, text).map_err(|err| Error::Io(format!("{}: {err}", part.display())))?;
    if let Err(err) = fs::rename(&part, destination) {
        let _ = fs::remove_file(&part);
        return Err(Error::Io(format!("{}: {err}", destination.display())));
    }
    Ok(destination.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread;

    const VALID: &str = "<datafile><header><name>T</name></header>\
<game name=\"G\"><rom name=\"g.nes\" size=\"1\" crc=\"00000000\"/></game></datafile>";

    fn serve_once(status: &str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let status = status.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        format!("http://127.0.0.1:{port}/catalog.dat")
    }

    #[test]
    fn downloads_and_replaces_a_valid_dat() {
        let url = serve_once("200 OK", VALID);
        let dir = std::env::temp_dir().join(format!("unko-fetch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let destination = dir.join("catalog.dat");
        let path = fetch_dat(&url, &destination).expect("download");
        assert_eq!(path, destination);
        assert_eq!(fs::read_to_string(&destination).expect("read"), VALID);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_dat_leaves_the_destination_untouched() {
        let url = serve_once("200 OK", "this is not a DAT");
        let dir = std::env::temp_dir().join(format!("unko-fetch-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let destination = dir.join("catalog.dat");
        fs::write(&destination, "old contents").expect("old");
        assert!(fetch_dat(&url, &destination).is_err());
        assert_eq!(
            fs::read_to_string(&destination).expect("read"),
            "old contents"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_error_is_reported() {
        let url = serve_once("404 Not Found", "missing");
        let dir = std::env::temp_dir().join(format!("unko-fetch-404-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let destination = dir.join("catalog.dat");
        let error = fetch_dat(&url, &destination).expect_err("must fail");
        assert!(error.to_string().contains("404"), "{error}");
        assert!(!destination.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
