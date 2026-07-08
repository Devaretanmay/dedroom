//! HTTPS CONNECT proxy tunnel support.
//!
//! Handles HTTP CONNECT requests so that any client using `HTTPS_PROXY` env
//! var routes through the DedrooM proxy. Traffic passes through in a raw TCP
//! tunnel (encrypted) — loop detection and compression cannot inspect the
//! content, but routing works for agents that don't respect the standard
//! `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL` env vars (e.g. OpenCode Zen).
//!
//! Listens on a separate port (default 8081) from the main HTTP proxy (8080).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Maximum CONNECT request header size (4 KB).
const MAX_CONNECT_HEADER: usize = 4096;

/// Entry point: handle a single CONNECT tunnel connection.
pub async fn handle_tunnel(stream: TcpStream) {
    match handle_connect(stream).await {
        Ok(()) => {}
        Err(e) => {
            tracing::debug!("CONNECT tunnel error: {e}");
        }
    }
}

/// Handle a single CONNECT tunnel request.
///
/// Reads the CONNECT request, resolves the target host:port, establishes
/// a TCP connection to it, sends back a 200, then copies bytes
/// bidirectionally (tunnel mode).
async fn handle_connect(mut stream: TcpStream) -> Result<(), String> {
    // Peek at the first bytes to check for "CONNECT "
    let mut buf = [0u8; 9];
    let peeked = stream.peek(&mut buf).await.map_err(|e| e.to_string())?;

    if peeked < 7 || &buf[..7] != b"CONNECT" {
        return Err("Not a CONNECT request".to_string());
    }

    // It's a CONNECT request — read the full request until \r\n\r\n
    let mut req_buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 1];
    loop {
        let n = stream.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("Connection closed while reading CONNECT request".to_string());
        }
        req_buf.push(tmp[0]);
        if req_buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if req_buf.len() > MAX_CONNECT_HEADER {
            return Err("CONNECT request too large".to_string());
        }
    }

    // Parse "CONNECT host:port HTTP/1.1"
    let request = String::from_utf8_lossy(&req_buf);
    let request_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("Malformed CONNECT request: {request_line}"));
    }

    let host_port = parts[1];

    // Resolve host:port — lookup_host handles IP:port, hostname:port, etc.
    let addr = tokio::net::lookup_host(host_port)
        .await
        .map_err(|e| format!("DNS resolution failed for {host_port}: {e}"))?
        .next()
        .ok_or_else(|| format!("No addresses found for {host_port}"))?;

    // Connect to the target
    let upstream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("Failed to connect to {addr}: {e}"))?;

    // Send 200 Connection Established
    let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
    stream.write_all(response).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;

    // Bidirectional copy (tunnel mode) — use into_split() for owned halves
    let (mut ri, mut wi) = stream.into_split();
    let (mut ru, mut wu) = upstream.into_split();

    let client_to_upstream = tokio::spawn(async move {
        tokio::io::copy(&mut ri, &mut wu).await.ok();
    });

    let upstream_to_client = tokio::spawn(async move {
        tokio::io::copy(&mut ru, &mut wi).await.ok();
    });

    // Wait for either direction to finish (connection closed)
    tokio::select! {
        _ = client_to_upstream => {},
        _ = upstream_to_client => {},
    }

    Ok(())
}
