//! DAT download and update.
//!
//! A downloaded DAT is validated as Logiqx XML before it replaces the
//! destination, so a bad URL, an HTML error page or a truncated transfer can
//! never clobber a working catalog. The transfer is bounded and uses a
//! temporary file in the destination directory followed by a rename.
//!
//! Only `https` URLs are accepted, except that plain `http` remains allowed
//! for loopback hosts (`localhost`, `127.0.0.0/8` and `::1`) so local test
//! servers keep working. The same rule is applied to the URL a redirect chain
//! finally settles on, so a request cannot be downgraded to cleartext. The
//! caller may also pin the expected SHA-256 of the downloaded DAT.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{parse_dat_str, Error, Result};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
use crate::hash::hex;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use sha2::{Digest, Sha256};

/// Largest accepted DAT transfer (256 MiB).
pub const MAX_DAT_BYTES: u64 = 256 * 1024 * 1024;

/// Whether `url` may be downloaded: `https` anywhere, or plain `http` only for
/// loopback hosts.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn scheme_allowed(url: &reqwest::Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => url.host_str().is_some_and(is_loopback_host),
        _ => false,
    }
}

/// Whether `host` names the local machine.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

/// Downloads `url` and atomically replaces `destination` with it.
///
/// The body must parse as a Logiqx DAT; otherwise the destination is left
/// untouched. The URL must use `https`, except for loopback hosts where plain
/// `http` is allowed; final URLs reached through redirects are checked against
/// the same rule. When `sha256` is `Some`, the validated body must match that
/// digest or the destination is left untouched.
///
/// # Errors
///
/// Returns [`Error::Http`] for a non-TLS URL, transport/status failures or a
/// redirect to a disallowed scheme, [`Error::Dat`] when the body is not a DAT
/// or does not match a pinned digest, and [`Error::Io`] when the file cannot be
/// written.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn fetch_dat(url: &str, destination: &Path, sha256: Option<&[u8; 32]>) -> Result<PathBuf> {
    use std::io::Read as _;
    use std::time::Duration;

    let parsed = reqwest::Url::parse(url)
        .map_err(|err| Error::Http(format!("{url}: invalid URL: {err}")))?;
    if !scheme_allowed(&parsed) {
        return Err(Error::Http(format!(
            "{url}: TLS (https) is required for non-loopback hosts"
        )));
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("unko-rom-manager/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| Error::Http(err.to_string()))?;
    let response = client
        .get(parsed)
        .send()
        .map_err(|err| Error::Http(format!("{url}: {err}")))?;
    let final_url = response.url();
    if !scheme_allowed(final_url) {
        return Err(Error::Http(format!(
            "{url}: redirect to {final_url} is not allowed; TLS (https) is required for non-loopback hosts"
        )));
    }
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
    let text = std::str::from_utf8(&body)
        .map_err(|_| Error::Dat(format!("{url}: DAT is not valid UTF-8")))?;
    let dat = parse_dat_str(text)
        .map_err(|err| Error::Dat(format!("{url}: downloaded DAT is invalid: {err}")))?;
    if dat.games.is_empty() {
        return Err(Error::Dat(format!(
            "{url}: downloaded DAT contains no games (not a DAT?)"
        )));
    }

    if let Some(expected) = sha256 {
        let actual: [u8; 32] = Sha256::digest(&body).into();
        if &actual != expected {
            return Err(Error::Dat(format!(
                "{url}: SHA-256 mismatch: expected {}, got {}",
                hex(expected),
                hex(&actual)
            )));
        }
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
    fs::write(&part, &body).map_err(|err| Error::Io(format!("{}: {err}", part.display())))?;
    if let Err(err) = fs::rename(&part, destination) {
        let _ = fs::remove_file(&part);
        return Err(Error::Io(format!("{}: {err}", destination.display())));
    }
    Ok(destination.to_path_buf())
}

/// DAT download is unavailable on mobile targets, which do not build a TLS
/// stack.
///
/// # Errors
///
/// Always returns [`Error::Http`] on iOS and Android.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn fetch_dat(_url: &str, _destination: &Path, _sha256: Option<&[u8; 32]>) -> Result<PathBuf> {
    Err(Error::Http(
        "DAT download is not available on this platform".into(),
    ))
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "android"))))]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
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
        let path = fetch_dat(&url, &destination, None).expect("download");
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
        assert!(fetch_dat(&url, &destination, None).is_err());
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
        let error = fetch_dat(&url, &destination, None).expect_err("must fail");
        assert!(error.to_string().contains("404"), "{error}");
        assert!(!destination.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_url_for_non_loopback_is_rejected() {
        let dir = std::env::temp_dir().join(format!("unko-fetch-tls-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let destination = dir.join("catalog.dat");
        let error = fetch_dat("http://example.com/catalog.dat", &destination, None)
            .expect_err("plain http off loopback must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("TLS") || message.contains("https"),
            "{message}"
        );
        assert!(!destination.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loopback_http_is_accepted_by_the_scheme_check() {
        // Bind then release a port so the connect attempt fails fast but the
        // URL still passes the scheme check.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let url = format!("http://127.0.0.1:{port}/x.dat");
        assert!(scheme_allowed(&reqwest::Url::parse(&url).expect("url")));
        let dir = std::env::temp_dir().join(format!("unko-fetch-loop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let destination = dir.join("catalog.dat");
        let error = fetch_dat(&url, &destination, None).expect_err("must fail to connect");
        let message = error.to_string();
        assert!(
            !message.contains("TLS"),
            "scheme check rejected loopback: {message}"
        );
        assert!(!destination.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn final_url_scheme_check() {
        let parse = |url: &str| reqwest::Url::parse(url).expect("url");
        assert!(scheme_allowed(&parse("https://example.com/catalog.dat")));
        assert!(scheme_allowed(&parse("http://127.0.0.1:8080/catalog.dat")));
        assert!(scheme_allowed(&parse("http://localhost/catalog.dat")));
        assert!(scheme_allowed(&parse("http://[::1]:8080/catalog.dat")));
        assert!(!scheme_allowed(&parse("http://example.com/catalog.dat")));
        assert!(!scheme_allowed(&parse("file:///tmp/catalog.dat")));
    }

    #[test]
    fn pinned_sha256_accepts_a_matching_download() {
        let url = serve_once("200 OK", VALID);
        let dir = std::env::temp_dir().join(format!("unko-fetch-pin-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let destination = dir.join("catalog.dat");
        let expected: [u8; 32] = Sha256::digest(VALID.as_bytes()).into();
        let path = fetch_dat(&url, &destination, Some(&expected)).expect("download");
        assert_eq!(path, destination);
        assert_eq!(fs::read_to_string(&destination).expect("read"), VALID);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pinned_sha256_rejects_a_mismatch_without_touching_the_destination() {
        let url = serve_once("200 OK", VALID);
        let dir = std::env::temp_dir().join(format!("unko-fetch-pin-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let destination = dir.join("catalog.dat");
        fs::write(&destination, b"old contents").expect("old");
        let before = fs::read(&destination).expect("before");
        let mut wrong: [u8; 32] = Sha256::digest(VALID.as_bytes()).into();
        wrong[0] ^= 0xff;
        let error = fetch_dat(&url, &destination, Some(&wrong)).expect_err("must fail");
        assert!(error.to_string().contains("SHA-256"), "{error}");
        assert_eq!(fs::read(&destination).expect("after"), before);
        let _ = fs::remove_dir_all(&dir);
    }
}
