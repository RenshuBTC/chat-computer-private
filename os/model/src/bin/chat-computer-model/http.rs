//! Just enough HTTP/1.1 for the model's server on this computer: a request (its line, headers and body, all
//! bounded), and answers: whole, or streamed as server-sent events.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// A request's body may be this big (a long conversation, with a screenshot's worth of text in it).
pub const MOST_BODY: usize = 64 << 20;
const MOST_HEAD: usize = 64 << 10;

pub struct Request {
    pub method: String,
    pub path: String,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

pub fn read(stream: &TcpStream) -> Result<Request, String> {
    let mut r = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut head = 0usize;
    let mut line = String::new();
    let mut next_line = |r: &mut BufReader<TcpStream>, line: &mut String| -> Result<(), String> {
        line.clear();
        let n = r.by_ref().take((MOST_HEAD - head) as u64).read_line(line).map_err(|e| e.to_string())?;
        head += n;
        if n == 0 || !line.ends_with('\n') {
            return Err("the request ended early, or its head is too long".into());
        }
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        Ok(())
    };
    next_line(&mut r, &mut line)?;
    let mut parts = line.split(' ');
    let (Some(method), Some(path), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
        return Err("not an HTTP request".into());
    };
    if !version.starts_with("HTTP/1.") {
        return Err("not HTTP/1".into());
    }
    let (method, path) = (method.to_string(), path.to_string());
    let mut headers = Vec::new();
    loop {
        next_line(&mut r, &mut line)?;
        if line.is_empty() {
            break;
        }
        let (n, v) = line.split_once(':').ok_or("a header without a colon")?;
        headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
        if headers.len() > 100 {
            return Err("too many headers".into());
        }
    }
    let mut req = Request { method, path, headers, body: Vec::new() };
    if req.header("transfer-encoding").is_some() {
        return Err("chunked requests aren't taken".into());
    }
    let len: usize = req.header("content-length").map(|v| v.parse().map_err(|_| "a bad length")).transpose()?.unwrap_or(0);
    if len > MOST_BODY {
        return Err("the request is too big".into());
    }
    req.body = vec![0; len];
    r.read_exact(&mut req.body).map_err(|e| e.to_string())?;
    Ok(req)
}

/// The headers every answer has: no caching, closed after, and (for a page allowed to ask) CORS.
fn common(cors: Option<&str>) -> String {
    let mut h = String::from("Cache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n");
    if let Some(origin) = cors {
        h.push_str(&format!("Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n"));
    }
    h
}

pub fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8], cors: Option<&str>) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    };
    let head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{}\r\n", body.len(), common(cors));
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A CORS preflight's answer, for a page that may ask (and Chrome's question about reaching this computer from a
/// public site: yes, for that page).
pub fn preflight(stream: &mut TcpStream, origin: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\nAccess-Control-Allow-Private-Network: true\r\n\
         Access-Control-Max-Age: 600\r\nContent-Length: 0\r\n{}\r\n",
        common(Some(origin))
    );
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

/// Server-sent events: the head, then each event as it comes.
pub struct Events<'a> {
    stream: &'a mut TcpStream,
}

impl<'a> Events<'a> {
    pub fn start(stream: &'a mut TcpStream, cors: Option<&str>) -> std::io::Result<Events<'a>> {
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{}\r\n", common(cors));
        stream.write_all(head.as_bytes())?;
        stream.flush()?;
        Ok(Events { stream })
    }

    pub fn send(&mut self, data: &str) -> std::io::Result<()> {
        self.stream.write_all(b"data: ")?;
        self.stream.write_all(data.as_bytes())?;
        self.stream.write_all(b"\n\n")?;
        self.stream.flush()
    }
}
