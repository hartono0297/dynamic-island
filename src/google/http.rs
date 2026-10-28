//! Blocking JSON/form HTTP over WinRT `HttpClient`. Every call waits on the
//! async operation with `.join()`, so call it only from worker threads.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use windows::Foundation::Uri;
use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions, UnicodeEncoding};
use windows::Win32::Foundation::{E_FAIL, ERROR_TIMEOUT};
use windows::Web::Http::Filters::{
    HttpBaseProtocolFilter, HttpCacheReadBehavior, HttpCacheWriteBehavior,
};
use windows::Web::Http::Headers::HttpCredentialsHeaderValue;
use windows::Web::Http::{
    HttpClient, HttpCompletionOption, HttpMethod, HttpRequestMessage, HttpResponseMessage,
    HttpStringContent,
};
use windows::core::{Error, HSTRING, Result};

/// Waits on a WinRT async operation until `deadline` (an `Option<Instant>`;
/// `None` waits as long as it takes). Past it the operation is cancelled and
/// the result is `ERROR_TIMEOUT`. Use inside a function returning `Result`.
macro_rules! join_by {
    ($op:expr, $deadline:expr) => {{
        let op = $op;
        match $deadline {
            None => op.join(),
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                let (done, finished) = mpsc::channel();
                op.when(move |_| {
                    let _ = done.send(());
                })?;
                if let Err(RecvTimeoutError::Timeout) = finished.recv_timeout(left) {
                    let _ = op.Cancel();
                    Err(Error::from(ERROR_TIMEOUT.to_hresult()))
                } else {
                    op.GetResults()
                }
            }
        }
    }};
}

pub struct Http {
    client: HttpClient,
    /// A request not answered by then is cancelled and fails; `None` waits.
    timeout: Option<Duration>,
}

impl Http {
    pub fn new() -> Result<Self> {
        // Never cache: responses carry tokens and calendar data, and stale event
        // lists would hide edits made elsewhere.
        let filter = HttpBaseProtocolFilter::new()?;
        let cache = filter.CacheControl()?;
        cache.SetReadBehavior(HttpCacheReadBehavior::NoCache)?;
        cache.SetWriteBehavior(HttpCacheWriteBehavior::NoCache)?;
        filter.SetAllowUI(false)?;
        let client = HttpClient::Create(&filter)?;
        Ok(Http {
            client,
            timeout: None,
        })
    }

    /// Every request fails once `timeout` passes without an answer.
    pub fn with_timeout(self, timeout: Duration) -> Self {
        Http {
            timeout: Some(timeout),
            ..self
        }
    }

    /// GET; returns the status and body. A non-2xx status is not an `Err`.
    pub fn get_json(&self, url: &str, bearer: Option<&str>) -> Result<(u16, String)> {
        let request = self.request(&HttpMethod::Get()?, url, bearer)?;
        self.send(&request)
    }

    /// GET with extra request headers (e.g. `User-Agent`), added without
    /// WinRT's header-value validation, which rejects some valid agents, for
    /// a body that must stay under `max_bytes`: it is read in pieces and the
    /// call fails (`Err`) as soon as it grows past that, so a hostile server
    /// cannot make it hold more. The body is read as UTF-8.
    pub fn get_with_headers_capped(
        &self,
        url: &str,
        extra: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<(u16, String)> {
        let request = self.request(&HttpMethod::Get()?, url, None)?;
        let headers = request.Headers()?;
        for (name, value) in extra {
            headers.TryAppendWithoutValidation(&HSTRING::from(*name), &HSTRING::from(*value))?;
        }
        // One deadline (`self.timeout`) for the headers and every read: the
        // body streams in pieces, so a stall at any point fails the call.
        let deadline = self.timeout.map(|t| Instant::now() + t);
        let response = join_by!(
            self.client
                .SendRequestWithOptionAsync(&request, HttpCompletionOption::ResponseHeadersRead)?,
            deadline
        )?;
        let status = response.StatusCode()?.0 as u16;
        let stream = join_by!(response.Content()?.ReadAsInputStreamAsync()?, deadline)?;
        const CHUNK: u32 = 64 * 1024;
        let mut body = Vec::new();
        loop {
            let buffer = join_by!(
                stream.ReadAsync(&Buffer::Create(CHUNK)?, CHUNK, InputStreamOptions::Partial)?,
                deadline
            )?;
            let len = buffer.Length()? as usize;
            if len == 0 {
                break;
            }
            if body.len() + len > max_bytes {
                return Err(Error::new(E_FAIL, "response too large"));
            }
            let start = body.len();
            body.resize(start + len, 0);
            DataReader::FromBuffer(&buffer)?.ReadBytes(&mut body[start..])?;
        }
        Ok((status, String::from_utf8_lossy(&body).into_owned()))
    }

    /// `method` is "POST" or "PATCH"; `body` is sent as UTF-8 JSON.
    pub fn send_json(
        &self,
        method: &str,
        url: &str,
        bearer: Option<&str>,
        body: &str,
    ) -> Result<(u16, String)> {
        let method = match method {
            "PATCH" => HttpMethod::Patch()?,
            "POST" => HttpMethod::Post()?,
            other => HttpMethod::Create(&HSTRING::from(other))?,
        };
        let request = self.request(&method, url, bearer)?;
        request.SetContent(
            &HttpStringContent::CreateFromStringWithEncodingAndMediaType(
                &HSTRING::from(body),
                UnicodeEncoding::Utf8,
                &HSTRING::from("application/json"),
            )?,
        )?;
        self.send(&request)
    }

    /// POST an `application/x-www-form-urlencoded` body (the OAuth token endpoint).
    pub fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)> {
        let body = form_encode(form);
        let request = self.request(&HttpMethod::Post()?, url, None)?;
        request.SetContent(
            &HttpStringContent::CreateFromStringWithEncodingAndMediaType(
                &HSTRING::from(body),
                UnicodeEncoding::Utf8,
                &HSTRING::from("application/x-www-form-urlencoded"),
            )?,
        )?;
        self.send(&request)
    }

    fn request(
        &self,
        method: &HttpMethod,
        url: &str,
        bearer: Option<&str>,
    ) -> Result<HttpRequestMessage> {
        let request = HttpRequestMessage::Create(method, &Uri::CreateUri(&HSTRING::from(url))?)?;
        let headers = request.Headers()?;
        headers.Append(&HSTRING::from("Accept"), &HSTRING::from("application/json"))?;
        if let Some(token) = bearer {
            headers.SetAuthorization(&HttpCredentialsHeaderValue::CreateFromSchemeWithToken(
                &HSTRING::from("Bearer"),
                &HSTRING::from(token),
            )?)?;
        }
        Ok(request)
    }

    /// `SendRequestAsync`, waited on until `self.timeout` at most: past it
    /// the operation is cancelled and the call fails with `ERROR_TIMEOUT`.
    /// The default completion reads the whole body before it completes, so
    /// the deadline covers a body that stalls too.
    fn send_request(&self, request: &HttpRequestMessage) -> Result<HttpResponseMessage> {
        let op = self.client.SendRequestAsync(request)?;
        let Some(timeout) = self.timeout else {
            return op.join();
        };
        let (done, finished) = mpsc::channel();
        op.when(move |_| {
            let _ = done.send(());
        })?;
        if let Err(RecvTimeoutError::Timeout) = finished.recv_timeout(timeout) {
            let _ = op.Cancel();
            return Err(Error::from(ERROR_TIMEOUT.to_hresult()));
        }
        op.GetResults()
    }

    fn send(&self, request: &HttpRequestMessage) -> Result<(u16, String)> {
        let response = self.send_request(request)?;
        let status = response.StatusCode()?.0 as u16;
        let body = response.Content()?.ReadAsStringAsync()?.join()?;
        Ok((status, body.to_string_lossy()))
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    /// A local server that answers each connection with `reply` after
    /// `delay` (holding the socket open meanwhile).
    fn serve_once(delay: Duration, reply: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                std::thread::sleep(delay);
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        url
    }

    #[test]
    fn a_stalled_request_fails_at_the_deadline_and_a_quick_one_answers() {
        let http = Http::new().unwrap().with_timeout(Duration::from_millis(400));
        let url = serve_once(Duration::from_secs(8), "");
        let start = Instant::now();
        assert!(http.get_json(&url, None).is_err());
        assert!(start.elapsed() < Duration::from_secs(4), "{:?}", start.elapsed());

        let url = serve_once(
            Duration::ZERO,
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
        );
        assert_eq!(http.get_json(&url, None).unwrap(), (200, "hi".to_owned()));
    }
}

/// `k=v&k=v` with both sides percent-encoded (query strings and form bodies).
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decodes `%XX` escapes and `+` (form encoding); malformed escapes are kept as-is.
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escape = bytes
            .get(i + 1..i + 3)
            .filter(|h| bytes[i] == b'%' && h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (bytes[i], escape) {
            (_, Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (b, None) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
