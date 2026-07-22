mod http;
mod pool;

use http::{ParseError, Request, Response};
use pool::ThreadPool;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::io::ErrorKind;
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;
use std::{fs, thread};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUESTS_PER_CONN: usize = 100;

fn build_tls_config() -> Arc<ServerConfig> {
    let certs = load_certs("cert.pem");
    let key = load_key("key.pem");
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("cert and key should be valid and match");
    Arc::new(config)
}

fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
    let file = fs::File::open(path).unwrap_or_else(|_| panic!("missing {path}"));
    let mut reader = BufReader::new(file);
    rustls_pemfile::certs(&mut reader)
        .map(|c| c.expect("valid certificate"))
        .collect()
}

fn load_key(path: &str) -> PrivateKeyDer<'static> {
    let file = fs::File::open(path).unwrap_or_else(|_| panic!("missing {path}"));
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .expect("readable key file")
        .expect("a private key in the file")
}

fn serve_tls(tcp: TcpStream, config: Arc<ServerConfig>) -> std::io::Result<()> {
    let conn =
        ServerConnection::new(config).map_err(|e| std::io::Error::new(ErrorKind::Other, e))?;
    let tls = StreamOwned::new(conn, tcp);
    handle_client(tls)
}

fn spawn_redirect_listener(addr: String, https_port: u16) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("http redirect listener failed to bind {addr}: {e}");
                return;
            }
        };
        for stream in listener.incoming().flatten() {
            let _ = redirect_to_https(stream, https_port);
        }
    });
}

fn redirect_to_https(mut stream: TcpStream, https_port: u16) -> std::io::Result<()> {
    let request = {
        let mut reader = BufReader::new(&stream);
        match http::parse_request(&mut reader) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        }
    };

    let host = request.header("host").unwrap_or("localhost");
    let host = host.split(':').next().unwrap_or(host);
    let location = if https_port == 443 {
        format!("https://{host}{}", request.path)
    } else {
        format!("https://{host}:{https_port}{}", request.path)
    };

    let response = format!(
        "HTTP/1.1 301 Moved Permanently\r\n\
         Location: {location}\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes())
}

fn handle_client(stream: impl Read + Write) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
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
                let mut response = Response::error(400, "Bad Request");
                response.set_close();
                response.write_to(reader.get_mut())?;
                return Ok(());
            }
        };

        count += 1;
        let keep_alive = request.is_keep_alive() && count < MAX_REQUESTS_PER_CONN;

        let mut response = catch_unwind(AssertUnwindSafe(|| serve(&request)))
            .unwrap_or_else(|_| Response::error(500, "Internal Server Error"));

        if !keep_alive {
            response.set_close();
        }

        response.write_to(reader.get_mut())?;

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
    let raw_path = request
        .path
        .split_once('?')
        .map(|(p, _)| p)
        .unwrap_or(&request.path);

    let path = if raw_path == "/" {
        "/index.html"
    } else {
        raw_path
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
    let https_port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8443);

    let addr = format!("127.0.0.1:{}", https_port);
    let workers = std::env::var("WORKERS")
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or_else(|| {
            thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });

    spawn_redirect_listener("127.0.0.1:8080".to_string(), https_port);

    let listener = TcpListener::bind(&addr)?;

    let pool = ThreadPool::new(workers);

    let tls_config = build_tls_config();

    for stream in listener.incoming() {
        match stream {
            Ok(tcp) => {
                let _ = tcp.set_read_timeout(Some(READ_TIMEOUT));
                let config = Arc::clone(&tls_config);
                pool.execute(move || {
                    if let Err(e) = serve_tls(tcp, config) {
                        eprintln!("connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept failed: {e}"),
        }
    }

    Ok(())
}
