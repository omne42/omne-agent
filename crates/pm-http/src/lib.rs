use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode};
use axum::routing::{any, get};
use http_body_util::BodyExt;
use pm_core::{FsStorage, PmPaths, RepositoryName, SessionId, Storage};
use pm_git::{RepoManager, lock_exclusive, lock_shared};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use tracing::{debug, info, warn};

#[derive(Clone)]
struct AppState {
    repos_root: PathBuf,
    repo_manager: RepoManager,
    storage: FsStorage,
}

pub fn router(pm_paths: PmPaths) -> anyhow::Result<Router> {
    let repos_root = pm_paths.repos_dir();
    let storage = FsStorage::new(pm_paths.data_dir());
    let repo_manager = RepoManager::new(pm_paths);

    let state = Arc::new(AppState {
        repos_root,
        repo_manager,
        storage,
    });

    Ok(Router::new()
        .route("/api/v0/repos", get(api_list_repos))
        .route("/api/v0/sessions", get(api_list_sessions))
        .route("/api/v0/sessions/:id/session", get(api_get_session))
        .route("/api/v0/sessions/:id/tasks", get(api_get_tasks))
        .route("/api/v0/sessions/:id/prs", get(api_get_prs))
        .route("/api/v0/sessions/:id/merge", get(api_get_merge))
        .route("/api/v0/sessions/:id/result", get(api_get_result))
        .route("/git/*path", any(git_http_backend))
        .with_state(state))
}

pub async fn serve(pm_paths: PmPaths, addr: SocketAddr) -> anyhow::Result<()> {
    let app = router(pm_paths)?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    info!(addr = %local_addr, "pm-http listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn api_list_repos(State(state): State<Arc<AppState>>) -> Result<Response<Body>, ApiError> {
    let repos = state.repo_manager.list_repos().await?;
    let json = serde_json::to_vec(&repos)?;
    Ok(json_response(json))
}

async fn api_list_sessions(State(state): State<Arc<AppState>>) -> Result<Response<Body>, ApiError> {
    let sessions = state.storage.list_session_ids().await?;
    let json = serde_json::to_vec(&sessions)?;
    Ok(json_response(json))
}

async fn api_get_session(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<SessionId>,
) -> Result<Response<Body>, ApiError> {
    api_get_json(
        &state.storage,
        &format!("sessions/{session_id}/session"),
        "session not found",
    )
    .await
}

async fn api_get_tasks(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<SessionId>,
) -> Result<Response<Body>, ApiError> {
    api_get_json(
        &state.storage,
        &format!("sessions/{session_id}/tasks"),
        "tasks not found",
    )
    .await
}

async fn api_get_prs(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<SessionId>,
) -> Result<Response<Body>, ApiError> {
    api_get_json(
        &state.storage,
        &format!("sessions/{session_id}/prs"),
        "prs not found",
    )
    .await
}

async fn api_get_merge(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<SessionId>,
) -> Result<Response<Body>, ApiError> {
    api_get_json(
        &state.storage,
        &format!("sessions/{session_id}/merge"),
        "merge not found",
    )
    .await
}

async fn api_get_result(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<SessionId>,
) -> Result<Response<Body>, ApiError> {
    api_get_json(
        &state.storage,
        &format!("sessions/{session_id}/result"),
        "result not found",
    )
    .await
}

async fn api_get_json(
    storage: &FsStorage,
    key: &str,
    not_found: &'static str,
) -> Result<Response<Body>, ApiError> {
    let value = storage.get_json(key).await?;
    match value {
        Some(value) => Ok(json_response(serde_json::to_vec(&value)?)),
        None => Err(ApiError::not_found(not_found)),
    }
}

fn json_response(bytes: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("internal error")))
}

async fn git_http_backend(
    State(state): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ApiError> {
    let (req_parts, req_body) = req.into_parts();
    let (repo_dir, tail) = split_repo_path(&path)?;
    validate_repo_dir(repo_dir)?;
    validate_git_path(tail)?;

    let repo_name = RepositoryName::new(repo_dir.trim_end_matches(".git").to_string())
        .map_err(|_| ApiError::not_found("unknown repo"))?;
    let repo_path = state.repos_root.join(format!("{}.git", repo_name.as_str()));
    if !tokio::fs::try_exists(&repo_path).await? {
        return Err(ApiError::not_found("unknown repo"));
    }
    let lock_path = state.repo_manager.paths().repo_lock_path(&repo_name);

    let method = req_parts.method.to_string();
    let query = req_parts.uri.query().unwrap_or("").to_string();
    let path_info = if tail.is_empty() {
        format!("/{repo_dir}")
    } else {
        format!("/{repo_dir}/{tail}")
    };

    let expect = req_parts
        .headers
        .get(axum::http::header::EXPECT)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("-");

    let content_type = req_parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let header_content_length = req_parts
        .headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let git_protocol = req_parts
        .headers
        .get("git-protocol")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    debug!(
        method = %req_parts.method,
        uri = %req_parts.uri,
        expect = %expect,
        content_length = %header_content_length.as_deref().unwrap_or("-"),
        content_type = %content_type.as_deref().unwrap_or("-"),
        git_protocol = %git_protocol.as_deref().unwrap_or("-"),
        "git smart http request"
    );

    let (content_length, body) = if method == "POST" && header_content_length.is_none() {
        let (path, len) = spool_body_to_tempfile(req_body).await?;
        (Some(len.to_string()), RequestBody::TempFile { path })
    } else {
        (header_content_length, RequestBody::Stream(req_body))
    };

    let is_write = method == "POST" && tail == "git-receive-pack";
    let repo_lock = if is_write {
        lock_exclusive(&lock_path).await?
    } else {
        lock_shared(&lock_path).await?
    };

    let mut command = tokio::process::Command::new("git");
    command
        .arg("http-backend")
        .current_dir(&state.repos_root)
        .env("GIT_PROJECT_ROOT", &state.repos_root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("PATH_INFO", path_info)
        .env("QUERY_STRING", query)
        .env("REQUEST_METHOD", &method)
        .env("SCRIPT_NAME", "/git")
        .env("SERVER_PROTOCOL", "HTTP/1.1")
        .env("SERVER_SOFTWARE", "code-pm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(value) = content_type {
        command.env("CONTENT_TYPE", value);
    }
    if let Some(value) = content_length {
        command.env("CONTENT_LENGTH", value);
    }
    if let Some(value) = git_protocol {
        command.env("HTTP_GIT_PROTOCOL", value);
    }

    let mut child = command.spawn().map_err(|err| {
        warn!(error = %err, "spawn git http-backend failed");
        ApiError::internal("spawn git http-backend failed")
    })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal("missing child stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::internal("missing child stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ApiError::internal("missing child stderr"))?;

    tokio::spawn(async move {
        let result: Result<(), std::io::Error> = async {
            match body {
                RequestBody::Stream(mut body) => {
                    while let Some(frame) = body.frame().await {
                        let frame =
                            frame.map_err(|_| std::io::Error::other("request body read failed"))?;
                        if let Some(data) = frame.data_ref() {
                            stdin.write_all(data).await?;
                        }
                    }
                    Ok(())
                }
                RequestBody::TempFile { path } => {
                    let mut file = tokio::fs::File::open(&path).await?;
                    tokio::io::copy(&mut file, &mut stdin).await?;
                    if let Err(err) = path.close() {
                        warn!(error = %err, "remove spooled request body failed");
                    }
                    Ok(())
                }
            }
        }
        .await;
        drop(stdin);

        if let Err(err) = &result {
            warn!(error = %err, "write git http-backend stdin failed");
        }
    });

    tokio::spawn(async move {
        let mut stderr = tokio::io::BufReader::new(stderr);
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf).await;
        if !buf.trim().is_empty() {
            debug!(stderr = %buf.trim(), "git http-backend stderr");
        }
    });

    let mut reader = tokio::io::BufReader::new(stdout);
    let (status, headers) = read_cgi_headers(&mut reader).await?;

    let body_stream = ReaderStream::new(reader);

    tokio::spawn(async move {
        let _repo_lock = repo_lock;
        match child.wait().await {
            Ok(status) => debug!(status = %status, "git http-backend exited"),
            Err(err) => warn!(error = %err, "git http-backend wait failed"),
        }
    });

    let mut response = Response::new(Body::from_stream(body_stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

enum RequestBody {
    Stream(Body),
    TempFile { path: tempfile::TempPath },
}

async fn spool_body_to_tempfile(body: Body) -> Result<(tempfile::TempPath, usize), ApiError> {
    let temp = tempfile::Builder::new()
        .prefix("code-pm-http-body-")
        .tempfile()?;
    let path = temp.into_temp_path();
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .await?;
    let mut body = body;
    let mut len: usize = 0;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ApiError::internal("request body read failed"))?;
        if let Some(data) = frame.data_ref() {
            len = len
                .checked_add(data.len())
                .ok_or_else(|| ApiError::internal("request body too large"))?;
            file.write_all(data).await?;
        }
    }
    file.flush().await?;

    Ok((path, len))
}

fn split_repo_path(path: &str) -> Result<(&str, &str), ApiError> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Err(ApiError::not_found("missing repo path"));
    }
    match path.split_once('/') {
        Some((repo, tail)) => Ok((repo, tail)),
        None => Ok((path, "")),
    }
}

fn validate_repo_dir(repo_dir: &str) -> Result<(), ApiError> {
    if !repo_dir.ends_with(".git") {
        return Err(ApiError::not_found("invalid repo path"));
    }
    let name = repo_dir.trim_end_matches(".git");
    if name.is_empty() {
        return Err(ApiError::not_found("invalid repo path"));
    }
    if RepositoryName::new(name.to_string()).is_err() {
        return Err(ApiError::not_found("invalid repo path"));
    }
    Ok(())
}

fn validate_git_path(path: &str) -> Result<(), ApiError> {
    if path.is_empty() {
        return Ok(());
    }
    if path.contains('\\') {
        return Err(ApiError::not_found("invalid repo path"));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(ApiError::not_found("invalid repo path"));
        }
    }
    Ok(())
}

async fn read_cgi_headers<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<(StatusCode, HeaderMap), ApiError> {
    let mut status = StatusCode::OK;
    let mut headers = HeaderMap::new();

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            if let Some(code) = value.split_whitespace().next() {
                if let Ok(code) = code.parse::<u16>() {
                    if let Ok(sc) = StatusCode::from_u16(code) {
                        status = sc;
                    }
                }
            }
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| ApiError::internal("bad header"))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|_| ApiError::internal("bad header value"))?;
        headers.append(header_name, header_value);
    }

    Ok((status, headers))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn not_found(message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }

    fn internal(message: &'static str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        warn!(error = %err, "pm-http internal error");
        Self::internal("internal error")
    }
}

impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        warn!(error = %err, "pm-http io error");
        Self::internal("io error")
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        warn!(error = %err, "pm-http json error");
        Self::internal("json error")
    }
}

impl From<ApiError> for Response<Body> {
    fn from(val: ApiError) -> Self {
        Response::builder()
            .status(val.status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(Body::from(val.message))
            .unwrap_or_else(|_| Response::new(Body::from("internal error")))
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        Response::from(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_repo_path_parses_first_segment() {
        assert_eq!(
            split_repo_path("repo.git/info/refs").unwrap(),
            ("repo.git", "info/refs")
        );
        assert_eq!(split_repo_path("/repo.git").unwrap(), ("repo.git", ""));
    }

    #[tokio::test]
    async fn spool_body_to_tempfile_writes_payload() -> anyhow::Result<()> {
        let body = Body::from("hello");
        let (path, len) = spool_body_to_tempfile(body)
            .await
            .map_err(|err| anyhow::anyhow!("spool failed: {err:?}"))?;

        assert_eq!(len, 5);
        let bytes = tokio::fs::read(&path).await?;
        assert_eq!(bytes, b"hello");

        path.close()?;
        Ok(())
    }
}
