use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, Write};

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub version: String,
    pub headers: HashMap<String, String>,
}

impl Request {
    pub fn is_keep_alive(&self) -> bool {
        match self.header("connection") {
            Some(v) if v.eq_ignore_ascii_case("close") => false,
            Some(v) if v.eq_ignore_ascii_case("keep-alive") => true,
            _ => self.version != "HTTP/1.0",
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }
}

#[derive(Debug)]
pub enum ParseError {
    Empty,
    BadRequest,
    BadHeader,
    Io(io::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty request"),
            ParseError::BadRequest => write!(f, "malformed request"),
            ParseError::BadHeader => write!(f, "malformed header"),
            ParseError::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl From<io::Error> for ParseError {
    fn from(e: io::Error) -> Self {
        ParseError::Io(e)
    }
}

pub fn parse_request(reader: impl BufRead) -> Result<Request, ParseError> {
    let mut lines = reader.lines();

    let request_line = lines.next().ok_or(ParseError::Empty)??;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(ParseError::BadRequest)?.to_string();
    let path = parts.next().ok_or(ParseError::BadRequest)?.to_string();
    let version = parts.next().unwrap_or("HTTP/1.0").to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let line = line?;
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(ParseError::BadHeader)?;
        headers
            .entry(name.trim().to_lowercase())
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(value.trim());
            })
            .or_insert_with(|| value.trim().to_string());
    }

    Ok(Request {
        method,
        path,
        version,
        headers,
    })
}

pub struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    connection_close: bool,
}

impl Response {
    pub fn set_close(&mut self) {
        self.connection_close = true;
    }

    pub fn new(
        status: u16,
        reason: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> Self {
        Response {
            status,
            reason,
            content_type,
            body,
            connection_close: false,
        }
    }

    pub fn ok(content_type: &'static str, body: Vec<u8>) -> Self {
        Response::new(200, "OK", content_type, body)
    }

    pub fn error(status: u16, reason: &'static str) -> Self {
        Response::new(
            status,
            reason,
            "text/html",
            format!("<h1>{status} {reason}</h1>").into_bytes(),
        )
    }

    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", self.status, self.reason);

        head.push_str("Strict-Transport-Security: max-age=300\r\n");

        if self.connection_close {
            head.push_str("Connection: close\r\n");
        }

        head.push_str(&format!(
            "Content-Type: {}\r\nContent-Length: {}\r\n\r\n",
            self.content_type,
            self.body.len()
        ));

        writer.write_all(head.as_bytes())?;
        writer.write_all(&self.body)
    }
}
