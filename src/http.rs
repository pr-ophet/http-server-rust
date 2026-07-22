use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, Write};

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }
}

#[derive(Debug)]
pub enum ParseError {
    Empty,
    BadRequestLine,
    BadHeader,
    Io(io::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty request"),
            ParseError::BadRequestLine => write!(f, "malformed request line"),
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
    let method = parts.next().ok_or(ParseError::BadRequestLine)?.to_string();
    let path = parts.next().ok_or(ParseError::BadRequestLine)?.to_string();

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
        headers,
    })
}

pub struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    pub fn ok(content_type: &'static str, body: Vec<u8>) -> Self {
        Response {
            status: 200,
            reason: "OK",
            content_type,
            body,
        }
    }

    pub fn error(status: u16, reason: &'static str) -> Self {
        Response {
            status,
            reason,
            content_type: "text/html",
            body: format!("<h1>{status} {reason}</h1>").into_bytes(),
        }
    }

    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        );
        writer.write_all(head.as_bytes())?;
        writer.write_all(&self.body)
    }
}
