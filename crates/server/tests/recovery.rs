//! Pruebas de recuperación con el binario `onepackd` real: caídas abruptas (SIGKILL / abort)
//! y reinicios (Fase 2, ADR-006).

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use zip::write::SimpleFileOptions;

const BIN: &str = env!("CARGO_BIN_EXE_onepackd");
const BOUNDARY: &str = "onepack-recovery";

fn onepackd(args: &[&str], data: &Path) {
    let status = Command::new(BIN)
        .args(args)
        .arg("--data-dir")
        .arg(data)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "onepackd {args:?} falló");
}

fn init(data: &Path) {
    onepackd(&["migrate"], data);
    onepackd(&["feed", "create", "internal"], data);
}

struct Server {
    child: Child,
    addr: SocketAddr,
}

impl Server {
    fn start(data: &Path, envs: &[(&str, &str)]) -> Self {
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let child = Command::new(BIN)
            .args([
                "serve",
                "--listen",
                &addr.to_string(),
                "--gc-grace-secs",
                "0",
                "--gc-interval-secs",
                "3600",
            ])
            .args([
                "--public-url",
                &format!("http://{addr}"),
                "--min-free-space-mib",
                "0",
            ])
            .arg("--data-dir")
            .arg(data)
            .envs(envs.iter().copied())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = Self { child, addr };
        wait_until(|| {
            server
                .request("GET", "/nuget/internal/v3/index.json", &[])
                .is_some_and(|(s, _)| s == 200)
        });
        server
    }

    /// Simula una caída abrupta (SIGKILL en Unix).
    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }

    fn request(&self, method: &str, path: &str, body: &[u8]) -> Option<(u16, Vec<u8>)> {
        let mut stream = TcpStream::connect(self.addr).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let content_type = if body.is_empty() {
            String::new()
        } else {
            format!("Content-Type: multipart/form-data; boundary={BOUNDARY}\r\n")
        };
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{content_type}Content-Length: {}\r\n\r\n",
            self.addr,
            body.len()
        );
        stream.write_all(head.as_bytes()).ok()?;
        stream.write_all(body).ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).ok()?;
        let split = response.windows(4).position(|w| w == b"\r\n\r\n")?;
        let status = std::str::from_utf8(&response[9..12]).ok()?.parse().ok()?;
        Some((status, response[split + 4..].to_vec()))
    }

    fn push(&self, package: &[u8]) -> Option<u16> {
        self.request("PUT", "/nuget/internal/v2/package", &multipart(package))
            .map(|(s, _)| s)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn multipart(package: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.nupkg\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(package);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn nupkg(id: &str, version: &str, padding: usize) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(format!("{id}.nuspec"), SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        format!(
            "<package><metadata><id>{id}</id><version>{version}</version></metadata></package>"
        )
        .as_bytes(),
    )
    .unwrap();
    // Contenido incompresible para controlar el tamaño del paquete.
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("content.bin", stored).unwrap();
    let mut x: u32 = 0x9e37_79b9;
    let noise: Vec<u8> = (0..padding)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    zip.write_all(&noise).unwrap();
    zip.finish().unwrap().into_inner()
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "la condición no se cumplió a tiempo"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn files_in(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_in(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn published_bytes_survive_restart() {
    let data = TempDir::new().unwrap();
    init(data.path());
    let package = nupkg("Hemia.Logging", "1.0.0", 10_000);

    let server = Server::start(data.path(), &[]);
    assert_eq!(server.push(&package), Some(201));
    server.kill();

    let server = Server::start(data.path(), &[]);
    let (status, body) = server
        .request(
            "GET",
            "/nuget/internal/v3/flat/hemia.logging/1.0.0/hemia.logging.1.0.0.nupkg",
            &[],
        )
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body, package);
}

#[test]
fn crash_during_upload_leaves_no_partial_version() {
    let data = TempDir::new().unwrap();
    init(data.path());
    let package = nupkg("Partial", "1.0.0", 4 << 20);
    let body = multipart(&package);

    let server = Server::start(data.path(), &[]);
    // Envía las cabeceras y la mitad del cuerpo, y mata el servidor a mitad de la subida.
    let mut stream = TcpStream::connect(server.addr).unwrap();
    let head = format!(
        "PUT /nuget/internal/v2/package HTTP/1.1\r\nHost: x\r\nContent-Type: multipart/form-data; boundary={BOUNDARY}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&body[..body.len() / 2]).unwrap();
    let staging = data.path().join("staging");
    wait_until(|| {
        files_in(&staging)
            .iter()
            .any(|f| f.metadata().is_ok_and(|m| m.len() > 0))
    });
    server.kill();
    drop(stream);
    assert!(
        !files_in(&staging).is_empty(),
        "la subida interrumpida deja un archivo en staging"
    );

    let server = Server::start(data.path(), &[]);
    wait_until(|| files_in(&staging).is_empty());
    assert_eq!(
        server
            .request("GET", "/nuget/internal/v3/flat/partial/index.json", &[])
            .unwrap()
            .0,
        404,
        "no debe existir una versión parcial"
    );
    assert_eq!(server.push(&package), Some(201), "la identidad sigue libre");
}

#[cfg(feature = "fault-injection")]
#[test]
fn crash_between_blob_persist_and_commit_leaves_recoverable_orphan() {
    let data = TempDir::new().unwrap();
    init(data.path());
    let package = nupkg("Orphan", "1.0.0", 1_000);
    let blobs = data.path().join("blobs");

    let server = Server::start(
        data.path(),
        &[("ONEPACK_FAULT_ABORT_AFTER_BLOB_PERSIST", "1")],
    );
    assert_eq!(
        server.push(&package),
        None,
        "el proceso aborta antes de responder"
    );
    let mut child = server;
    assert!(!child.child.wait().unwrap().success());
    assert_eq!(
        files_in(&blobs).len(),
        1,
        "el blob quedó persistido sin metadatos"
    );

    let server = Server::start(data.path(), &[]);
    wait_until(|| files_in(&blobs).is_empty());
    assert_eq!(
        server
            .request("GET", "/nuget/internal/v3/flat/orphan/index.json", &[])
            .unwrap()
            .0,
        404
    );
    assert_eq!(server.push(&package), Some(201));
    assert_eq!(files_in(&blobs).len(), 1);
}
