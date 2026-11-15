//! Stopping the server binary while clients hold their streams open.
#![allow(clippy::unwrap_used)] // helpers outside #[test] fns
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn connect(address: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

/// Sends one request and returns the response head, leaving the body unread.
fn request(address: SocketAddr, head: &str, body: &str) -> (String, BufReader<TcpStream>) {
    let mut stream = connect(address);
    write!(
        stream,
        "{head}\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    while !response.ends_with("\r\n\r\n") {
        assert_ne!(reader.read_line(&mut response).unwrap(), 0, "{response}");
    }
    (response, reader)
}

/// Reads a chunked event stream until `needle` shows up.
fn read_until(reader: &mut BufReader<TcpStream>, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        assert_ne!(reader.read_line(&mut seen).unwrap(), 0, "{seen}");
    }
    seen
}

const MCP: &str = "Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-06-18";

/// Opens the stream an MCP client keeps for server messages.
fn mcp_stream(address: SocketAddr) -> BufReader<TcpStream> {
    let (head, _) = request(
        address,
        &format!("POST /mcp HTTP/1.1\r\n{MCP}"),
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let session = head
        .lines()
        .find_map(|line| line.strip_prefix("mcp-session-id: "))
        .unwrap_or_else(|| panic!("no session: {head}"))
        .to_owned();
    let (head, _) = request(
        address,
        &format!("POST /mcp HTTP/1.1\r\n{MCP}\r\nMcp-Session-Id: {session}"),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert!(head.starts_with("HTTP/1.1 202"), "{head}");
    let (head, stream) = request(
        address,
        &format!("GET /mcp HTTP/1.1\r\n{MCP}\r\nMcp-Session-Id: {session}"),
        "",
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    stream
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn a_stop_signal_ends_the_server_while_streams_are_open() {
    let camp = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_blender-live"))
            .arg(camp.path())
            .env("PORT", address.port().to_string())
            .env("LOG_LEVEL", "info")
            .env("TMPDIR", tmp.path())
            .env(
                "BLENDER_BIN",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_blender.py"),
            )
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).is_err() {
        assert!(server.0.try_wait().unwrap().is_none(), "server exited");
        assert!(Instant::now() < deadline, "server did not start");
        thread::sleep(Duration::from_millis(20));
    }

    // What a browser tab and an MCP client leave open.
    let (head, mut events) = request(address, "GET /api/events HTTP/1.1", "");
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let status = read_until(&mut events, "\"pid\":");
    let blender = status
        .split("\"pid\":")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .unwrap()
        .to_owned();
    assert!(alive(&blender), "no Blender at pid {blender}");
    let _mcp = mcp_stream(address);

    // systemd signals every process of the unit at once. Blender is gone at
    // once, while a busy server may get to its own signal a moment later.
    let server_pid = server.0.id().to_string();
    for pid in [&blender, &server_pid] {
        let signalled = Command::new("kill").args(["-TERM", pid]).status().unwrap();
        assert!(signalled.success(), "kill {pid} failed");
        thread::sleep(Duration::from_millis(100));
    }

    let stopping = Instant::now();
    let exit = loop {
        if let Some(exit) = server.0.try_wait().unwrap() {
            break exit;
        }
        assert!(
            stopping.elapsed() < Duration::from_secs(8),
            "still running 8 s after SIGTERM"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(exit.success(), "{exit}");

    let mut log = String::new();
    server
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    assert!(
        log.matches("Blender ready").count() == 1 && !log.contains("was restarted"),
        "Blender was started again while stopping:\n{log}"
    );
    assert!(!alive(&blender), "Blender outlived the server");
    let left: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|item| item.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "temporary files stayed: {left:?}");
}
