mod config;
mod http;
mod pool;

use config::Config;
use http::{ParseError, Request, Response};
use pool::ThreadPool;
use std::fs;
use std::io::BufReader;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUESTS_PER_CONN: usize = 100;

fn handle_client(stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(&stream);
    let mut count = 0usize;

    loop {
        let request = match http::parse_request(&mut reader) {
            Ok(request) => request,
            Err(ParseError::Empty) => return Ok(()),
            Err(ParseError::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                return Ok(());
            }
            Err(e) => {
                eprintln!("bad request: {e}");
                Response::error(400, "Bad Request").write_to(&mut (&stream))?;
                return Ok(());
            }
        };

        count += 1;
        let keep_alive = request.is_keep_alive() && count < MAX_REQUESTS_PER_CONN;

        println!("{} {}", request.method, request.path);

        let mut response = catch_unwind(AssertUnwindSafe(|| serve(&request)))
            .unwrap_or_else(|_| Response::error(500, "Internal Server Error"));

        if !keep_alive {
            response.set_close();
        }

        response.write_to(&mut (&stream))?;

        if !keep_alive {
            return Ok(());
        }
    }
}

fn not_found() -> Response {
    match fs::read("html/404.html") {
        Ok(body) => Response::new(404, "Not Found", "text/html", body),
        Err(_) => Response::error(404, "Not Found"),
    }
}

fn serve(request: &Request) -> Response {
    if request.method != "GET" {
        return Response::error(405, "Method Not Allowed");
    }

    let path = if request.path == "/" {
        "/index.html"
    } else {
        request.path.as_str()
    };

    if path.contains("..") {
        return not_found();
    }

    let file_path = format!("html{path}");
    match fs::read(&file_path) {
        Ok(body) => Response::ok(content_type_for(path), body),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::IsADirectory) => not_found(),
        Err(e) => {
            eprintln!("error reading {file_path}: {e}");
            Response::error(500, "Internal Server Error")
        }
    }
}

fn content_type_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn main() -> std::io::Result<()> {
    let config = Config::load();

    let listener = TcpListener::bind(&config.addr)?;
    println!("listening on {}", listener.local_addr()?);

    let pool = ThreadPool::new(config.workers);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => pool.execute(move || {
                if let Err(e) = handle_client(stream) {
                    eprintln!("connection error: {e}");
                }
            }),
            Err(e) => eprintln!("accept failed: {e}"),
        }
    }

    Ok(())
}
