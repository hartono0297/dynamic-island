//! Blocking JSON/form HTTP over WinRT `HttpClient`. Every call waits on the
//! async operation with `.join()`, so call it only from worker threads.

use windows::Foundation::Uri;
use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions, UnicodeEncoding};
use windows::Win32::Foundation::E_FAIL;
use windows::Web::Http::Filters::{
    HttpBaseProtocolFilter, HttpCacheReadBehavior, HttpCacheWriteBehavior,
};
use windows::Web::Http::Headers::HttpCredentialsHeaderValue;
use windows::Web::Http::{
    HttpClient, HttpCompletionOption, HttpMethod, HttpRequestMessage, HttpStringContent,
};
use windows::core::{Error, HSTRING, Result};

pub struct Http {
    client: HttpClient,
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
        Ok(Http { client })
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
        let response = self
            .client
            .SendRequestWithOptionAsync(&request, HttpCompletionOption::ResponseHeadersRead)?
            .join()?;
        let status = response.StatusCode()?.0 as u16;
        let stream = response.Content()?.ReadAsInputStreamAsync()?.join()?;
        const CHUNK: u32 = 64 * 1024;
        let mut body = Vec::new();
        loop {
            let buffer = stream
                .ReadAsync(&Buffer::Create(CHUNK)?, CHUNK, InputStreamOptions::Partial)?
                .join()?;
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

    fn send(&self, request: &HttpRequestMessage) -> Result<(u16, String)> {
        let response = self.client.SendRequestAsync(request)?.join()?;
        let status = response.StatusCode()?.0 as u16;
        let body = response.Content()?.ReadAsStringAsync()?.join()?;
        Ok((status, body.to_string_lossy()))
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
