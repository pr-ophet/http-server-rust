use std::{env, thread};

pub struct Config {
    pub addr: String,
    pub workers: usize,
}

impl Config {
    pub fn load() -> Self {
        let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
        let workers = env::var("WORKERS")
            .ok()
            .and_then(|w| w.parse().ok())
            .unwrap_or_else(default_workers);

        Config {
            addr: format!("127.0.0.1:{port}"),
            workers,
        }
    }
}

fn default_workers() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
