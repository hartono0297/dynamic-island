//! Google OAuth for an installed app: PKCE (S256) with a loopback redirect,
//! token refresh, and a DPAPI-encrypted token file. Tokens, codes and the
//! client secret never appear in logs or error strings.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    BCRYPT_SHA256_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom, BCryptHash,
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use super::http::{Http, form_encode, url_decode};
use crate::config::{self, Config};

/// Read the calendar list (to know which calendars are `selected`) and
/// list/insert/patch events. Source: Google Calendar API v3 reference,
/// calendarList.list and events.insert "Authorization" sections.
pub const SCOPES: &str = "https://www.googleapis.com/auth/calendar.calendarlist.readonly https://www.googleapis.com/auth/calendar.events";

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// A stray connection (browser preconnect) that sends nothing is dropped after this.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const REFRESH_MARGIN_SECS: i64 = 60;
/// `parse_redirect` error for a request that is not the OAuth redirect (e.g. `/favicon.ico`).
const NO_CODE: &str = "no_code";

const PAGE_OK: &str = "<!doctype html><meta charset=\"utf-8\"><title>Dynamic Island</title>\
    <p style=\"font:16px sans-serif\">Berhasil terhubung. Kembali ke island ya.</p>";
const PAGE_FAILED: &str = "<!doctype html><meta charset=\"utf-8\"><title>Dynamic Island</title>\
    <p style=\"font:16px sans-serif\">Gagal terhubung. Coba lagi dari island ya.</p>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No client ID in the config, or Google rejected the client credentials.
    NotConfigured,
    /// No stored tokens (or the user cancelled the consent screen).
    NeedsLogin,
    /// The refresh token was revoked or expired (`invalid_grant`).
    Reconnect,
    Network(String),
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_at_unix: i64,
}

/// Redacted, so a stray `{:?}` cannot leak a token.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access", &"<redacted>")
            .field("refresh", &"<redacted>")
            .field("expires_at_unix", &self.expires_at_unix)
            .finish()
    }
}

/// Base64 with the URL-safe alphabet and no padding (RFC 4648 section 5).
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | u32::from(b) << (16 - 8 * i));
        // 1, 2 or 3 input bytes give 2, 3 or 4 output characters.
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// PKCE S256: `base64url(SHA-256(verifier))`.
pub fn pkce_challenge(verifier: &str) -> String {
    let mut digest = [0u8; 32];
    // The SHA-256 pseudo-handle needs no setup and does not fail on Windows 10+.
    // If it ever did, Google would reject the code exchange, which surfaces as an error.
    let status = unsafe {
        BCryptHash(
            BCRYPT_SHA256_ALG_HANDLE,
            None,
            verifier.as_bytes(),
            &mut digest,
        )
    };
    debug_assert!(status.is_ok(), "BCryptHash failed: {status:?}");
    base64url(&digest)
}

pub fn auth_url(client_id: &str, redirect: &str, challenge: &str, state: &str) -> String {
    let query = form_encode(&[
        ("client_id", client_id),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        // A refresh token on every consent, so a re-login always replaces a revoked one.
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", state),
    ]);
    format!("{AUTH_ENDPOINT}?{query}")
}

/// Parses the loopback request line, e.g. `GET /?state=..&code=.. HTTP/1.1`.
/// `Err` carries Google's `error` value, or `no_code` for any other request.
pub fn parse_redirect(request_line: &str) -> Result<(String, String), String> {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Err(NO_CODE.into());
    };
    let Some((_, query)) = target.split_once('?') else {
        return Err(NO_CODE.into());
    };
    let (mut code, mut state, mut error) = (None, None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match key {
            "code" => &mut code,
            "state" => &mut state,
            "error" => &mut error,
            _ => continue,
        };
        *slot = Some(url_decode(value));
    }
    if let Some(error) = error {
        return Err(error);
    }
    match (code, state) {
        (Some(code), Some(state)) => Ok((code, state)),
        _ => Err(NO_CODE.into()),
    }
}

/// Google omits `refresh_token` on refresh responses, so the old one is kept.
/// `None` when neither side has a refresh token.
pub fn merge_tokens(old: Option<&Tokens>, resp: TokenResponse, now: i64) -> Option<Tokens> {
    let refresh = resp
        .refresh_token
        .filter(|r| !r.is_empty())
        .or_else(|| old.map(|t| t.refresh.clone()))?;
    Some(Tokens {
        access: resp.access_token,
        refresh,
        expires_at_unix: now.saturating_add(i64::try_from(resp.expires_in).unwrap_or(i64::MAX)),
    })
}

/// Maps a token-endpoint failure using only its `error` / `error_description`
/// fields, so nothing else from the body ends up in a message.
pub fn classify_token_error(status: u16, body: &str) -> AuthError {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct ErrorBody {
        error: String,
        error_description: String,
    }
    let parsed: ErrorBody = serde_json::from_str(body).unwrap_or_default();
    match parsed.error.as_str() {
        "invalid_grant" => AuthError::Reconnect,
        "invalid_client" | "unauthorized_client" => AuthError::NotConfigured,
        "" => AuthError::Network(format!("HTTP {status}")),
        error if parsed.error_description.is_empty() => {
            AuthError::Network(format!("HTTP {status}: {error}"))
        }
        error => AuthError::Network(format!(
            "HTTP {status}: {error} ({})",
            parsed.error_description
        )),
    }
}

/// True when the access token expires within the refresh margin.
fn needs_refresh(tokens: &Tokens, now: i64) -> bool {
    tokens.expires_at_unix - now <= REFRESH_MARGIN_SECS
}

/// `n_bytes` from the system RNG, base64url encoded; `None` if the RNG fails.
fn try_random_urlsafe(n_bytes: usize) -> Option<String> {
    let mut buf = vec![0u8; n_bytes];
    let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    status.is_ok().then(|| base64url(&buf))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn network(e: windows::core::Error) -> AuthError {
    AuthError::Network(e.message())
}

fn io_error(e: io::Error) -> AuthError {
    AuthError::Network(e.to_string())
}

/// Interactive login: opens the consent page in the browser, waits up to five
/// minutes for the loopback redirect, exchanges the code and saves the tokens.
/// Blocks; run it on a worker thread.
pub fn login(http: &Http, client_id: &str, client_secret: &str) -> Result<Tokens, AuthError> {
    if client_id.trim().is_empty() {
        return Err(AuthError::NotConfigured);
    }
    let rng_failed = || AuthError::Network("system RNG failed".into());
    // 32 bytes give a 43-character verifier, the RFC 7636 minimum length.
    let verifier = try_random_urlsafe(32).ok_or_else(rng_failed)?;
    let state = try_random_urlsafe(16).ok_or_else(rng_failed)?;

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(io_error)?;
    let port = listener.local_addr().map_err(io_error)?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    open_browser(&auth_url(
        client_id,
        &redirect,
        &pkce_challenge(&verifier),
        &state,
    ))?;
    let code = wait_for_code(&listener, &state)?;
    drop(listener);

    let (status, body) = http
        .post_form(
            TOKEN_ENDPOINT,
            &[
                ("code", &code),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("redirect_uri", &redirect),
                ("grant_type", "authorization_code"),
                ("code_verifier", &verifier),
            ],
        )
        .map_err(network)?;
    let tokens = token_result(status, &body, None)?;
    save_tokens(&tokens)
        .map_err(|e| AuthError::Network(format!("could not save the token: {}", e.kind())))?;
    Ok(tokens)
}

pub fn refresh(
    http: &Http,
    client_id: &str,
    client_secret: &str,
    tokens: &Tokens,
) -> Result<Tokens, AuthError> {
    let (status, body) = http
        .post_form(
            TOKEN_ENDPOINT,
            &[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", &tokens.refresh),
                ("grant_type", "refresh_token"),
            ],
        )
        .map_err(network)?;
    token_result(status, &body, Some(tokens))
}

fn token_result(status: u16, body: &str, old: Option<&Tokens>) -> Result<Tokens, AuthError> {
    if !(200..300).contains(&status) {
        return Err(classify_token_error(status, body));
    }
    let resp: TokenResponse = serde_json::from_str(body)
        .map_err(|_| AuthError::Network("unreadable token response".into()))?;
    merge_tokens(old, resp, now_unix())
        .ok_or_else(|| AuthError::Network("Google sent no refresh token".into()))
}

/// A valid access token, refreshed (and saved) when it expires within a minute.
/// `Reconnect` also deletes the stored tokens.
pub fn access_token(http: &Http, cfg: &Config) -> Result<String, AuthError> {
    if cfg.google_client_id.trim().is_empty() {
        return Err(AuthError::NotConfigured);
    }
    let tokens = load_tokens().ok_or(AuthError::NeedsLogin)?;
    if !needs_refresh(&tokens, now_unix()) {
        return Ok(tokens.access);
    }
    match refresh(
        http,
        &cfg.google_client_id,
        &cfg.google_client_secret,
        &tokens,
    ) {
        Ok(fresh) => {
            // A failed save only costs another refresh next time.
            let _ = save_tokens(&fresh);
            Ok(fresh.access)
        }
        Err(AuthError::Reconnect) => {
            clear_tokens();
            Err(AuthError::Reconnect)
        }
        Err(e) => Err(e),
    }
}

/// ShellExecute wants COM on the calling thread; a short-lived STA thread
/// keeps that apartment choice away from the blocking sync worker.
fn open_browser(url: &str) -> Result<(), AuthError> {
    let url = HSTRING::from(url);
    let opened = std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let result = ShellExecuteW(None, w!("open"), &url, None, None, SW_SHOWNORMAL);
        if com.is_ok() {
            CoUninitialize();
        }
        // ShellExecute reports success as a value above 32.
        result.0 as isize > 32
    })
    .join()
    .unwrap_or(false);
    if opened {
        Ok(())
    } else {
        Err(AuthError::Network("could not open the browser".into()))
    }
}

/// Accepts loopback connections until the OAuth redirect arrives or the
/// login times out. Other requests (favicon, empty preconnects) are skipped.
fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, AuthError> {
    listener.set_nonblocking(true).map_err(io_error)?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(outcome) = answer_redirect(&stream, expected_state) {
                    return outcome;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(AuthError::Network("login timed out".into()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}

/// `None` when the request was not the redirect and waiting should go on.
fn answer_redirect(stream: &TcpStream, expected_state: &str) -> Option<Result<String, AuthError>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream.take(16 * 1024));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    // Drain the headers: closing with unread data resets the connection and
    // the browser would show an error instead of the page.
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok_and(|n| n > 0) && !header.trim_end().is_empty() {
        header.clear();
    }

    let outcome = match parse_redirect(request_line.trim_end()) {
        Err(e) if e == NO_CODE => {
            respond(stream, "404 Not Found", "");
            return None;
        }
        Err(e) if e == "access_denied" => Err(AuthError::NeedsLogin),
        Err(e) => Err(AuthError::Network(format!(
            "login error: {}",
            e.chars().take(64).collect::<String>()
        ))),
        Ok((_, state)) if state != expected_state => {
            Err(AuthError::Network("login state mismatch".into()))
        }
        Ok((code, _)) => Ok(code),
    };
    respond(
        stream,
        "200 OK",
        if outcome.is_ok() {
            PAGE_OK
        } else {
            PAGE_FAILED
        },
    );
    Some(outcome)
}

fn respond(mut stream: &TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

/// `%APPDATA%\DynamicIsland\google-token.bin`, next to the config file.
fn token_path() -> PathBuf {
    config::path().with_file_name("google-token.bin")
}

pub fn load_tokens() -> Option<Tokens> {
    load_tokens_from(&token_path())
}

pub fn save_tokens(tokens: &Tokens) -> io::Result<()> {
    save_tokens_to(&token_path(), tokens)
}

pub fn clear_tokens() {
    let _ = std::fs::remove_file(token_path());
}

fn load_tokens_from(path: &Path) -> Option<Tokens> {
    let sealed = std::fs::read(path).ok()?;
    let plain = unprotect(&sealed).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Written to a temp file and renamed, so a crash never leaves half a token file.
fn save_tokens_to(path: &Path, tokens: &Tokens) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let plain = serde_json::to_vec(tokens).map_err(io::Error::other)?;
    let sealed = protect(&plain)?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, sealed)?;
    std::fs::rename(&tmp, path)
}

/// DPAPI, current-user scope: only this Windows account can decrypt it.
fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(plain)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            w!("Dynamic Island Google token"),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(io::Error::other)?;
    Ok(unsafe { take_blob(output, false) })
}

fn unprotect(sealed: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(sealed)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(io::Error::other)?;
    Ok(unsafe { take_blob(output, true) })
}

fn blob(data: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let len = u32::try_from(data.len()).map_err(io::Error::other)?;
    // DPAPI only reads the input blob; `*mut` is just the C struct's field type.
    Ok(CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: data.as_ptr().cast_mut(),
    })
}

/// Copies a DPAPI output blob and frees it with `LocalFree`, zeroing it first
/// when it holds plaintext.
///
/// # Safety
/// `blob` must come from a successful `CryptProtectData`/`CryptUnprotectData`.
unsafe fn take_blob(blob: CRYPT_INTEGER_BLOB, wipe: bool) -> Vec<u8> {
    if blob.pbData.is_null() {
        return Vec::new();
    }
    let len = blob.cbData as usize;
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, len) }.to_vec();
    unsafe {
        if wipe {
            for i in 0..len {
                std::ptr::write_volatile(blob.pbData.add(i), 0);
            }
        }
        LocalFree(Some(HLOCAL(blob.pbData.cast())));
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            // RFC 7636 Appendix B (the plan's copy of the verifier had a typo).
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn base64url_has_no_padding() {
        assert_eq!(base64url(b"ab"), "YWI");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn auth_url_has_required_params() {
        let url = auth_url(
            "cid.apps.googleusercontent.com",
            "http://127.0.0.1:5555",
            "chal",
            "xyz",
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        for part in [
            "client_id=cid.apps.googleusercontent.com",
            "response_type=code",
            "code_challenge=chal",
            "code_challenge_method=S256",
            "access_type=offline",
            "prompt=consent",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555",
            "state=xyz",
        ] {
            assert!(url.contains(part), "missing {part} in {url}");
        }
        let scope = format!(
            "scope={}",
            SCOPES
                .replace(':', "%3A")
                .replace('/', "%2F")
                .replace(' ', "%20")
        );
        assert!(url.contains(&scope), "missing encoded scope in {url}");
    }

    #[test]
    fn parse_redirect_ok_and_error() {
        assert_eq!(
            parse_redirect("GET /?state=s1&code=4%2F0Ab HTTP/1.1"),
            Ok(("4/0Ab".to_string(), "s1".to_string()))
        );
        assert_eq!(
            parse_redirect("GET /?error=access_denied&state=s1 HTTP/1.1"),
            Err("access_denied".to_string())
        );
    }

    #[test]
    fn token_response_without_refresh_keeps_old_refresh() {
        let old = Tokens {
            access: "old-a".into(),
            refresh: "old-r".into(),
            expires_at_unix: 10,
        };
        let resp: TokenResponse = serde_json::from_str(
            r#"{"access_token":"new-a","expires_in":3599,"token_type":"Bearer"}"#,
        )
        .unwrap();
        let merged = merge_tokens(Some(&old), resp, 1_000).unwrap();
        assert_eq!(
            merged,
            Tokens {
                access: "new-a".into(),
                refresh: "old-r".into(),
                expires_at_unix: 4_599
            }
        );

        let fresh: TokenResponse =
            serde_json::from_str(r#"{"access_token":"a","expires_in":60}"#).unwrap();
        assert!(
            merge_tokens(None, fresh, 0).is_none(),
            "no refresh token at all"
        );
    }

    #[test]
    fn invalid_grant_maps_to_reconnect() {
        assert!(matches!(
            classify_token_error(400, r#"{"error":"invalid_grant"}"#),
            AuthError::Reconnect
        ));
    }

    #[test]
    fn refresh_only_within_a_minute_of_expiry() {
        let t = Tokens {
            access: "a".into(),
            refresh: "r".into(),
            expires_at_unix: 1_000,
        };
        assert!(!needs_refresh(&t, 939));
        assert!(needs_refresh(&t, 940));
        assert!(needs_refresh(&t, 2_000));
    }

    /// Sends one raw request to the loopback listener and returns the raw response.
    fn send_raw(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn loopback_run(requests: &'static [&'static str]) -> (Result<String, AuthError>, Vec<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            requests
                .iter()
                .map(|r| send_raw(port, r))
                .collect::<Vec<_>>()
        });
        let outcome = wait_for_code(&listener, "s1");
        (outcome, client.join().unwrap())
    }

    #[test]
    fn loopback_skips_other_requests_and_answers_the_redirect() {
        let (outcome, responses) = loopback_run(&[
            "GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "GET /?state=s1&code=4%2F0Ab&scope=x HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: */*\r\n\r\n",
        ]);
        assert_eq!(outcome, Ok("4/0Ab".to_string()));
        assert!(responses[0].starts_with("HTTP/1.1 404"));
        assert!(responses[1].starts_with("HTTP/1.1 200 OK"));
        assert!(responses[1].contains("Berhasil terhubung. Kembali ke island ya."));
    }

    #[test]
    fn loopback_rejects_a_mismatched_state() {
        let (outcome, responses) = loopback_run(&["GET /?state=evil&code=c HTTP/1.1\r\n\r\n"]);
        assert_eq!(
            outcome,
            Err(AuthError::Network("login state mismatch".into()))
        );
        assert!(responses[0].contains("Gagal terhubung"));
    }

    /// Live request with bogus credentials: checks the WinRT client, form
    /// encoding and error mapping. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs network"]
    fn live_bogus_credentials_are_classified() {
        let http = Http::new().unwrap();
        let bogus = Tokens {
            access: "x".into(),
            refresh: "x".into(),
            expires_at_unix: 0,
        };
        assert_eq!(
            refresh(&http, "bogus-client", "bogus-secret", &bogus),
            Err(AuthError::NotConfigured)
        );
        let (status, _) = http
            .get_json(
                "https://www.googleapis.com/calendar/v3/users/me/calendarList",
                Some("bogus"),
            )
            .unwrap();
        assert_eq!(status, 401);
    }

    #[test]
    fn dpapi_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "dynamic-island-token-test-{}.bin",
            std::process::id()
        ));
        let tokens = Tokens {
            access: "ya29.access-token-text".into(),
            refresh: "1//refresh-token-text".into(),
            expires_at_unix: 1_790_000_000,
        };
        save_tokens_to(&path, &tokens).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let loaded = load_tokens_from(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded, Some(tokens));
        let needle = b"access-token-text";
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle),
            "token stored in plain text"
        );
    }
}
