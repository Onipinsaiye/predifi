//! Mock Stellar RPC server for integration tests.
//!
//! Binds an ephemeral port and responds to every HTTP request with a minimal
//! JSON-RPC 2.0 payload so health probes succeed without hitting the real network.
//!
//! Also serves archived oracle provider vectors so the offline regression suite
//! can exercise resolution behavior without external network access.
//!
//! Supports fault injection (latency, connection resets, throttling) and
//! recovery so integration tests can exercise partial RPC outages.

use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use std::sync::Arc;
use std::time::Duration;

/// A running mock RPC server that can be cleanly shut down after a test.
pub struct MockRpcServer {
    url: String,
    shutdown_tx: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

/// A single archived oracle provider vector.
///
/// Vectors are redacted: no live endpoints, credentials, or account secrets are
/// stored. `provenance` records where the response shape was observed and
/// `expected_normalized` captures the reviewed normalized value the SDK must
/// produce. Any change to a vector requires review per `ORACLE_VECTORS.md`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OracleVector {
    /// Stable identifier used by contract and backend tests.
    pub id: String,
    /// Category: `valid`, `stale`, `malformed`, or `boundary`.
    pub category: String,
    /// Redacted provenance (provider name, observed date, redaction notes).
    pub provenance: String,
    /// Raw provider response body as returned over JSON-RPC.
    pub raw_response: serde_json::Value,
    /// Expected normalized value after SDK interpretation, or `null` if the
    /// response must be rejected.
    pub expected_normalized: serde_json::Value,
}

/// Load the archived oracle vectors embedded at compile time.
///
/// The corpus lives in `backend/src/test_support/oracle_vectors.json` and is
/// shared verbatim with the contract test suite so schema or interpretation
/// drift is detected on both sides.
pub fn load_oracle_vectors() -> Vec<OracleVector> {
    let raw = include_str!("oracle_vectors.json");
    serde_json::from_str(raw).expect("archived oracle vectors must be valid JSON")
}

/// Fault-injection mode applied to incoming requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcFault {
    /// Respond normally.
    None,
    /// Delay the response by the given duration before replying.
    Latency(Duration),
    /// Accept the connection then reset it without a response.
    ConnectionReset,
    /// Respond with HTTP 429 (throttled) and a JSON-RPC error body.
    Throttle,
}

/// Shared, mutable fault state so tests can flip modes mid-traffic.
#[derive(Clone)]
pub struct FaultHandle {
    inner: Arc<Mutex<RpcFault>>,
}

impl FaultHandle {
    /// Set the current fault mode. Takes effect on subsequent requests.
    pub async fn set(&self, fault: RpcFault) {
        *self.inner.lock().await = fault;
    }

    /// Read the current fault mode.
    pub async fn get(&self) -> RpcFault {
        *self.inner.lock().await
    }
}

impl MockRpcServer {
    /// Bind an ephemeral port and start accepting connections in the background.
    pub async fn start() -> Self {
        Self::start_with_fault(RpcFault::None).await
    }

    /// Bind an ephemeral port and start with an initial fault mode.
    pub async fn start_with_fault(initial: RpcFault) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind mock RPC listener");
        let addr = listener.local_addr().expect("failed to get local addr");
        let url = format!("http://{}", addr);

        let fault = FaultHandle {
            inner: Arc::new(Mutex::new(initial)),
        };
        let fault_for_loop = fault.clone();

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let vectors = Arc::new(load_oracle_vectors());

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    result = listener.accept() => {
                        if let Ok((mut socket, _)) = result {
                            let vectors = Arc::clone(&vectors);
                            let fault = fault_for_loop.clone();
                            tokio::spawn(async move {
                                let mut buf = [0u8; 1024];
                                let _ = socket.read(&mut buf).await;
                                let request = String::from_utf8_lossy(&buf);
                                let mode = fault.get().await;
                                match mode {
                                    RpcFault::None => {}
                                    RpcFault::Latency(d) => {
                                        tokio::time::sleep(d).await;
                                    }
                                    RpcFault::ConnectionReset => {
                                        // Drop the socket without writing a response.
                                        let _ = socket.shutdown().await;
                                        return;
                                    }
                                    RpcFault::Throttle => {
                                        let body = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"throttled"}}"#;
                                        let response = format!(
                                            "HTTP/1.1 429 Too Many Requests\r\n\
                                             Content-Type: application/json\r\n\
                                             Content-Length: {}\r\n\
                                             Connection: close\r\n\
                                             \r\n\
                                             {}",
                                            body.len(),
                                            body
                                        );
                                        let _ = socket.write_all(response.as_bytes()).await;
                                        let _ = socket.shutdown().await;
                                        return;
                                    }
                                }
                                let body = match select_vector(&request, &vectors) {
                                    Some(vector) => serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": 1,
                                        "result": vector.raw_response,
                                    })
                                    .to_string(),
                                    None => {
                                        r#"{"jsonrpc":"2.0","id":1,"result":{"status":"healthy"}}"#
                                            .to_string()
                                    }
                                };
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\n\
                                     Content-Type: application/json\r\n\
                                     Content-Length: {}\r\n\
                                     Connection: close\r\n\
                                     \r\n\
                                     {}",
                                    body.len(),
                                    body
                                );
                                let _ = socket.write_all(response.as_bytes()).await;
                                let _ = socket.shutdown().await;
                            });
                        }
                    }
                }
            }
        });

        Self {
            url,
            shutdown_tx,
            handle,
        }
    }

    /// Obtain a handle to mutate fault mode while the server is running.
    pub fn fault_handle(&self) -> FaultHandle {
        self.fault.clone()
    }

    /// Serve a specific archived vector by id, regardless of request body.
    ///
    /// Used by regression tests that need deterministic, offline provider
    /// responses. Returns the vector's expected normalized value.
    pub fn expected_normalized(&self, id: &str) -> Option<serde_json::Value> {
        load_oracle_vectors()
            .into_iter()
            .find(|vector| vector.id == id)
            .map(|vector| vector.expected_normalized)
    }

    /// Base URL to assign to [`crate::config::Config::stellar_rpc_url`].
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Stop the accept loop and wait for the background task to exit.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.handle.await;
    }
}

/// Match an incoming JSON-RPC request against archived vectors by `method`.
///
/// Requests that do not name a known vector fall back to the health payload so
/// existing probes keep working.
fn select_vector(request: &str, vectors: &[OracleVector]) -> Option<&OracleVector> {
    let parsed: serde_json::Value = serde_json::from_str(request).ok()?;
    let method = parsed.get("method")?.as_str()?;
    vectors.iter().find(|vector| {
        vector
            .raw_response
            .get("method")
            .and_then(|value| value.as_str())
            == Some(method)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_rpc_responds_to_health_probe() {
        let mock = MockRpcServer::start().await;
        let client = reqwest::Client::new();
        let response = client
            .post(mock.url())
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getHealth"
            }))
            .send()
            .await
            .expect("request should succeed");

        assert!(response.status().is_success());
        mock.shutdown().await;
    }

    #[test]
    fn archived_vectors_cover_all_categories() {
        let vectors = load_oracle_vectors();
        assert!(!vectors.is_empty(), "vector corpus must not be empty");
        for category in ["valid", "stale", "malformed", "boundary"] {
            assert!(
                vectors.iter().any(|vector| vector.category == category),
                "missing archived vector for category: {category}"
            );
        }
        for vector in &vectors {
            assert!(
                !vector.provenance.is_empty(),
                "vector {} must record redacted provenance",
                vector.id
            );
        }
    }

    #[tokio::test]
    async fn mock_rpc_serves_archived_vector_offline() {
        let mock = MockRpcServer::start().await;
        let vectors = load_oracle_vectors();
        let vector = vectors
            .iter()
            .find(|vector| vector.category == "valid")
            .expect("valid vector must exist");

        let client = reqwest::Client::new();
        let response = client
            .post(mock.url())
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": vector
                    .raw_response
                    .get("method")
                    .and_then(|value| value.as_str())
                    .unwrap_or("getHealth")
            }))
            .send()
            .await
            .expect("request should succeed");

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await.expect("valid JSON body");
        assert_eq!(
            body.get("result"),
            Some(&vector.raw_response),
            "mock must return the archived raw response"
        );
        assert_eq!(
            mock.expected_normalized(&vector.id),
            Some(vector.expected_normalized.clone())
        );
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn mock_rpc_injects_latency() {
        let mock = MockRpcServer::start_with_fault(RpcFault::Latency(Duration::from_millis(50))).await;
        let client = reqwest::Client::new();
        let start = std::time::Instant::now();
        let response = client
            .post(mock.url())
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getHealth"}))
            .send()
            .await
            .expect("request should succeed");
        assert!(response.status().is_success());
        assert!(start.elapsed() >= Duration::from_millis(50));
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn mock_rpc_throttles_then_recovers() {
        let mock = MockRpcServer::start_with_fault(RpcFault::Throttle).await;
        let fault = mock.fault_handle();
        let client = reqwest::Client::new();

        let throttled = client
            .post(mock.url())
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getHealth"}))
            .send()
            .await
            .expect("request should complete");
        assert_eq!(throttled.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);

        fault.set(RpcFault::None).await;
        let recovered = client
            .post(mock.url())
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getHealth"}))
            .send()
            .await
            .expect("request should succeed");
        assert!(recovered.status().is_success());
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn mock_rpc_resets_connection() {
        let mock = MockRpcServer::start_with_fault(RpcFault::ConnectionReset).await;
        let client = reqwest::Client::new();
        let result = client
            .post(mock.url())
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getHealth"}))
            .send()
            .await;
        assert!(result.is_err());
        mock.shutdown().await;
    }
        mock.shutdown().await;
    }
}
