use std::fs;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use wasm_bindgen_cli_support::Bindgen;

fn main() {
    if let Err(e) = run() {
        eprintln!("web-synth: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/web_synth_app");
    build_wasm(&crate_dir)?;

    let pkg_dir = crate_dir.join("pkg");
    fs::create_dir_all(&pkg_dir).map_err(|e| e.to_string())?;
    let wasm = crate_dir.join("target/wasm32-unknown-unknown/release/web_synth_app.wasm");

    Bindgen::new()
        .input_path(&wasm)
        .web(true)
        .map_err(|e| e.to_string())?
        .generate(&pkg_dir)
        .map_err(|e| e.to_string())?;

    fs::copy(crate_dir.join("index.html"), pkg_dir.join("index.html"))
        .map_err(|e| e.to_string())?;

    serve(&pkg_dir)
}

fn build_wasm(crate_dir: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .current_dir(crate_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .status()
        .map_err(|e| format!("failed to launch cargo: {e}"))?;
    if !status.success() {
        return Err("wasm build failed (is the wasm32-unknown-unknown target installed?)".into());
    }
    Ok(())
}

fn serve(dir: &Path) -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = format!("http://localhost:{port}");
    println!("web-synth serving at {url}");
    open_browser(&url);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle(stream, dir),
            Err(e) => eprintln!("web-synth: connection error: {e}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, dir: &Path) {
    let mut buffer = [0u8; 1024];
    let read = match stream.read(&mut buffer) {
        Ok(n) => n,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&buffer[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let relative = match path {
        "/" => "index.html",
        other => other.trim_start_matches('/'),
    };

    if relative.contains("..") {
        let _ = respond(&mut stream, "403 Forbidden", "text/plain", b"forbidden");
        return;
    }

    match fs::read(dir.join(relative)) {
        Ok(body) => {
            let _ = respond(&mut stream, "200 OK", content_type(relative), &body);
        }
        Err(_) => {
            let _ = respond(&mut stream, "404 Not Found", "text/plain", b"not found");
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8]) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "text/javascript"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else {
        "application/octet-stream"
    }
}

fn open_browser(url: &str) {
    let launcher = if cfg!(target_os = "macos") {
        Some(("open", vec![url]))
    } else if cfg!(target_os = "windows") {
        Some(("cmd", vec!["/C", "start", url]))
    } else {
        Some(("xdg-open", vec![url]))
    };
    if let Some((program, args)) = launcher {
        let _ = Command::new(program).args(args).spawn();
    }
}
