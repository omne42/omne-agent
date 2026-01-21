use std::ffi::{OsStr, OsString};
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

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

#[derive(Debug)]
pub struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout_lines: tokio::io::Lines<tokio::io::BufReader<ChildStdout>>,
    next_id: u64,
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
        let stdout_lines = tokio::io::BufReader::new(stdout).lines();

        Ok(Self {
            child,
            stdin,
            stdout_lines,
            next_id: 1,
        })
    }

    pub fn child_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        loop {
            let Some(line) = self.stdout_lines.next_line().await? else {
                return Err(Error::Protocol("server closed stdout".to_string()));
            };
            let resp: JsonRpcResponse = serde_json::from_str(&line)?;
            if resp.id != serde_json::json!(id) {
                return Err(Error::Protocol(format!(
                    "unexpected response id: expected {id}, got {}",
                    resp.id
                )));
            }
            if let Some(err) = resp.error {
                return Err(Error::Rpc {
                    code: err.code,
                    message: err.message,
                    data: err.data,
                });
            }
            let Some(result) = resp.result else {
                return Err(Error::Protocol("missing result".to_string()));
            };
            return Ok(result);
        }
    }

    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, Error> {
        Ok(self.child.wait().await.map_err(std::io::Error::from)?)
    }
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
