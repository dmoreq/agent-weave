//! HTTP request/response primitives: bounded parsing, loopback Host guard,
//! defensive headers, and one stderr line per request.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Instant;

pub(super) struct HttpRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) body: Vec<u8>,
    pub(super) content_type: String,
    pub(super) origin: Option<String>,
    pub(super) host: Option<String>,
}

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;

pub(super) fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(None);
    }

    let mut content_length = 0usize;
    let mut content_type = String::new();
    let mut origin = None;
    let mut host = None;
    let mut line = String::new();
    let mut header_bytes = request_line.len();
    let mut header_count = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        header_bytes += line.len();
        header_count += 1;
        if header_bytes > MAX_HEADER_BYTES || header_count > MAX_HEADERS {
            return Ok(None);
        }
        let lower = line.to_lowercase();
        let value = line.split_once(':').map(|(_, v)| v.trim().to_string());
        if let Some(val) = value {
            if lower.starts_with("content-length:") {
                content_length = val.parse().unwrap_or(0);
            } else if lower.starts_with("content-type:") {
                content_type = val.to_lowercase();
            } else if lower.starts_with("origin:") {
                origin = Some(val);
            } else if lower.starts_with("host:") {
                host = Some(val);
            }
        }
    }

    let mut body = Vec::new();
    if content_length > 0 && content_length < 1_000_000 {
        use std::io::Read;
        body.resize(content_length, 0);
        reader.read_exact(&mut body)?;
    }

    Ok(Some(HttpRequest {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        body,
        content_type,
        origin,
        host,
    }))
}

/// The dashboard answers only for loopback hostnames. A rebound public domain
/// resolves to 127.0.0.1 but still sends its own `Host`, which this rejects.
pub(super) fn host_allowed(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return true;
    };
    let name = host
        .split(':')
        .next()
        .unwrap_or(host)
        .trim_start_matches('[');
    matches!(name, "127.0.0.1" | "localhost" | "::1" | "0.0.0.0")
}

/// Loopback-only request authenticity: a mutating request must be
/// `application/json`, and a browser-supplied `Origin` must match the Host it
/// addressed. Non-browser clients (curl) may omit Origin.
pub(super) fn touch_request_allowed(req: &HttpRequest) -> std::result::Result<(), (u16, String)> {
    if !req.content_type.starts_with("application/json") {
        return Err((415, "Content-Type must be application/json".to_string()));
    }
    if let Some(origin) = &req.origin {
        let expected = req
            .host
            .as_deref()
            .map(|h| format!("http://{h}"))
            .unwrap_or_default();
        if origin != &expected {
            return Err((403, "cross-origin mutation rejected".to_string()));
        }
    }
    Ok(())
}

pub(super) struct RequestLog {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) started: Instant,
}

impl Drop for RequestLog {
    fn drop(&mut self) {
        eprintln!(
            "eg dashboard: {} {} {:.1}ms",
            self.method,
            self.path,
            self.started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Send one response with the dashboard's fixed defensive headers. Unknown
/// statuses are 500s, never mislabelled successes.
pub(super) fn send_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    is_head: bool,
) -> std::io::Result<()> {
    let status_line = match status {
        200 => "200 OK",
        400 => "400 Bad Request",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        409 => "409 Conflict",
        415 => "415 Unsupported Media Type",
        503 => "503 Service Unavailable",
        _ => "500 Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n\
         Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !is_head {
        stream.write_all(body)?;
    }
    stream.flush()
}
