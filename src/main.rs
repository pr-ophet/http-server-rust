mod config;
mod http;
mod pool;

use config::Config;
use http::{Request, Response};
use pool::ThreadPool;
use std::fs;
use std::io::BufReader;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

fn handle_client(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;

    let reader = BufReader::new(&mut stream);
    let response = match http::parse_request(reader) {
        Ok(request) => {
            println!(
                "{} {} ({})",
                request.method,
                request.path,
                request.header("user-agent").unwrap_or("-")
            );
            catch_unwind(AssertUnwindSafe(|| serve(&request))).unwrap_or_else(|_| {
                eprintln!("handler panicked on {} {}", request.method, request.path);
                Response::error(500, "Internal Server Error")
            })
        }
        Err(e) => {
            eprintln!("bad request: {e}");
            Response::error(400, "Bad Request")
        }
    };

    response.write_to(&mut stream)
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
        return Response::error(404, "Not Found");
    }

    let file_path = format!("html{path}");
    match fs::read(&file_path) {
        Ok(body) => Response::ok(content_type_for(path), body),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::IsADirectory) => {
            Response::error(404, "Not Found")
        }
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
