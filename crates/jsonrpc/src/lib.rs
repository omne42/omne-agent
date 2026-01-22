use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("json-rpc error {code}: {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("protocol error: {0}")]
    Protocol(String),
}

type PendingRequests = Arc<tokio::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, Error>>>>>;

#[derive(Debug)]
enum Transport {
    Child {
        child: Child,
        stdin: ChildStdin,
    },
    #[cfg(unix)]
    Unix {
        write: tokio::net::unix::OwnedWriteHalf,
    },
}

#[derive(Debug)]
pub struct Client {
    transport: Transport,
    next_id: u64,
    pending: PendingRequests,
    notifications_rx: Option<mpsc::UnboundedReceiver<Notification>>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    pub async fn spawn<I, S>(program: S, args: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = OsString>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::inherit());

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("child stdin not captured".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("child stdout not captured".to_string()))?;
        let (notify_tx, notify_rx) = mpsc::unbounded_channel::<Notification>();
        let pending: PendingRequests = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let task = spawn_reader_task(stdout, pending.clone(), notify_tx);

        Ok(Self {
            transport: Transport::Child { child, stdin },
            next_id: 1,
            pending,
            notifications_rx: Some(notify_rx),
            task,
        })
    }

    pub async fn connect_unix(path: &Path) -> Result<Self, Error> {
        #[cfg(unix)]
        {
            let stream = tokio::net::UnixStream::connect(path).await?;
            let (read, write) = stream.into_split();
            let (notify_tx, notify_rx) = mpsc::unbounded_channel::<Notification>();
            let pending: PendingRequests = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let task = spawn_reader_task(read, pending.clone(), notify_tx);
            Ok(Self {
                transport: Transport::Unix { write },
                next_id: 1,
                pending,
                notifications_rx: Some(notify_rx),
                task,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(Error::Protocol(
                "unix socket client is only supported on unix".to_string(),
            ))
        }
    }

    pub fn child_id(&self) -> Option<u32> {
        match &self.transport {
            Transport::Child { child, .. } => child.id(),
            #[cfg(unix)]
            Transport::Unix { .. } => None,
        }
    }

    pub fn take_notifications(&mut self) -> Option<mpsc::UnboundedReceiver<Notification>> {
        self.notifications_rx.take()
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let (tx, rx) = oneshot::channel::<Result<Value, Error>>();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        if let Err(err) = self.write_line(&line).await {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
            return Err(err);
        }

        match rx.await {
            Ok(result) => result,
            Err(_) => Err(Error::Protocol("response channel closed".to_string())),
        }
    }

    async fn write_line(&mut self, line: &str) -> Result<(), Error> {
        match &mut self.transport {
            Transport::Child { stdin, .. } => {
                stdin.write_all(line.as_bytes()).await?;
                stdin.flush().await?;
            }
            #[cfg(unix)]
            Transport::Unix { write } => {
                write.write_all(line.as_bytes()).await?;
                write.flush().await?;
            }
        }
        Ok(())
    }

    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, Error> {
        self.task.abort();
        match &mut self.transport {
            Transport::Child { child, .. } => Ok(child.wait().await?),
            #[cfg(unix)]
            Transport::Unix { .. } => {
                Err(Error::Protocol("client has no child process".to_string()))
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, serde::Deserialize)]
struct JsonRpcResponse {
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, serde::Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

fn spawn_reader_task<R>(
    reader: R,
    pending: PendingRequests,
    notify_tx: mpsc::UnboundedSender<Notification>,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(reader).lines();
        loop {
            let next = lines.next_line().await;
            match next {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let Some(method) = value
                        .get("method")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                    else {
                        if value.get("id").is_none() {
                            continue;
                        }
                        let response: JsonRpcResponse = match serde_json::from_value(value) {
                            Ok(resp) => resp,
                            Err(err) => {
                                drain_pending(
                                    &pending,
                                    Error::Protocol(format!("invalid response: {err}")),
                                )
                                .await;
                                return;
                            }
                        };

                        let Some(id) = response.id.as_u64() else {
                            continue;
                        };

                        let tx = {
                            let mut pending = pending.lock().await;
                            pending.remove(&id)
                        };
                        let Some(tx) = tx else {
                            continue;
                        };

                        if let Some(err) = response.error {
                            let _ = tx.send(Err(Error::Rpc {
                                code: err.code,
                                message: err.message,
                                data: err.data,
                            }));
                            continue;
                        }

                        let Some(result) = response.result else {
                            let _ = tx.send(Err(Error::Protocol("missing result".to_string())));
                            continue;
                        };
                        let _ = tx.send(Ok(result));
                        continue;
                    };

                    let params = value.get("params").cloned().unwrap_or(Value::Null);
                    let _ = notify_tx.send(Notification { method, params });
                }
                Ok(None) => {
                    drain_pending(
                        &pending,
                        Error::Protocol("server closed connection".to_string()),
                    )
                    .await;
                    return;
                }
                Err(err) => {
                    drain_pending(&pending, Error::Io(err)).await;
                    return;
                }
            }
        }
    })
}

async fn drain_pending(pending: &PendingRequests, err: Error) {
    let pending = {
        let mut pending = pending.lock().await;
        std::mem::take(&mut *pending)
    };

    for (_id, tx) in pending {
        let _ = tx.send(Err(Error::Protocol(err.to_string())));
    }
}
