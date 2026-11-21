//! JSON-RPC over lines, as Codex's `app-server` speaks it on its stdio:
//! requests that are answered, notifications, and requests from the other
//! side (an approval) that we answer.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

/// What came from the other side unasked.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// A request to answer with [`Rpc::respond`] under `id`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

pub struct Rpc {
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Pending,
    next: AtomicU64,
}

impl Rpc {
    /// Speaks over `reader` and `writer`; what comes unasked goes to the
    /// receiver, which ends when the reader does.
    pub fn new(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> (Arc<Rpc>, mpsc::UnboundedReceiver<Incoming>) {
        let pending: Pending = Arc::default();
        let (tx, rx) = mpsc::unbounded_channel();
        let reading = Arc::clone(&pending);
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(m) = serde_json::from_str::<Value>(&line) else {
                    tracing::debug!("not JSON from the agent: {line}");
                    continue;
                };
                let method = m.get("method").and_then(Value::as_str).map(str::to_owned);
                let id = m.get("id").cloned();
                match (method, id) {
                    (Some(method), Some(id)) => {
                        let params = m.get("params").cloned().unwrap_or(Value::Null);
                        let _ = tx.send(Incoming::Request { id, method, params });
                    }
                    (Some(method), None) => {
                        let params = m.get("params").cloned().unwrap_or(Value::Null);
                        let _ = tx.send(Incoming::Notification { method, params });
                    }
                    (None, Some(id)) => {
                        let Some(id) = id.as_u64() else { continue };
                        if let Some(answer) = reading.lock().await.remove(&id) {
                            let result = match m.get("error") {
                                Some(e) => Err(anyhow!("{}", error_text(e))),
                                None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = answer.send(result);
                        }
                    }
                    (None, None) => {}
                }
            }
            // The other side is gone: whoever waits hears so.
            for (_, answer) in reading.lock().await.drain() {
                let _ = answer.send(Err(anyhow!("the agent stopped")));
            }
        });
        let rpc = Rpc {
            writer: Mutex::new(Box::new(writer)),
            pending,
            next: AtomicU64::new(1),
        };
        (Arc::new(rpc), rx)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        self.send(json!({"id": id, "method": method, "params": params}))
            .await?;
        rx.await.map_err(|_| anyhow!("the agent stopped"))?
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"method": method, "params": params})).await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.send(json!({"id": id, "result": result})).await
    }

    async fn send(&self, message: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(&line).await?;
        writer.flush().await?;
        Ok(())
    }
}

fn error_text(e: &Value) -> String {
    e.get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| e.to_string())
}
