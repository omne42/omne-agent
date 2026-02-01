use serde::de::DeserializeOwned;

#[derive(Clone)]
struct DbVfsHttpClient {
    base_url: String,
    client: reqwest::Client,
}

impl DbVfsHttpClient {
    fn from_env() -> anyhow::Result<Option<Self>> {
        let raw = match std::env::var("OMNE_AGENT_DB_VFS_URL") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(err) => return Err(anyhow::anyhow!("read OMNE_AGENT_DB_VFS_URL: {err}")),
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        Ok(Some(Self {
            base_url: trimmed.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }))
    }

    async fn read(
        &self,
        workspace_id: String,
        path: String,
    ) -> Result<DbVfsReadResponse, DbVfsHttpError> {
        self.post_json(
            "/v1/read",
            &DbVfsReadRequest {
                workspace_id,
                path,
                start_line: None,
                end_line: None,
            },
        )
        .await
    }

    async fn write(
        &self,
        workspace_id: String,
        path: String,
        content: String,
        expected_version: Option<u64>,
    ) -> Result<DbVfsWriteResponse, DbVfsHttpError> {
        self.post_json(
            "/v1/write",
            &DbVfsWriteRequest {
                workspace_id,
                path,
                content,
                expected_version,
            },
        )
        .await
    }

    async fn delete(
        &self,
        workspace_id: String,
        path: String,
        ignore_missing: bool,
    ) -> Result<DbVfsDeleteResponse, DbVfsHttpError> {
        self.post_json(
            "/v1/delete",
            &DbVfsDeleteRequest {
                workspace_id,
                path,
                expected_version: None,
                ignore_missing,
            },
        )
        .await
    }

    async fn glob(
        &self,
        workspace_id: String,
        pattern: String,
        path_prefix: Option<String>,
    ) -> Result<DbVfsGlobResponse, DbVfsHttpError> {
        self.post_json(
            "/v1/glob",
            &DbVfsGlobRequest {
                workspace_id,
                pattern,
                path_prefix,
            },
        )
        .await
    }

    async fn grep(
        &self,
        workspace_id: String,
        query: String,
        regex: bool,
        glob: Option<String>,
        path_prefix: Option<String>,
    ) -> Result<DbVfsGrepResponse, DbVfsHttpError> {
        self.post_json(
            "/v1/grep",
            &DbVfsGrepRequest {
                workspace_id,
                query,
                regex,
                glob,
                path_prefix,
            },
        )
        .await
    }

    async fn post_json<Req: Serialize + ?Sized, Resp: DeserializeOwned>(
        &self,
        endpoint: &str,
        req: &Req,
    ) -> Result<Resp, DbVfsHttpError> {
        let url = format!("{}{}", self.base_url, endpoint);
        let response = self
            .client
            .post(url)
            .timeout(std::time::Duration::from_secs(30))
            .json(req)
            .send()
            .await
            .map_err(|err| DbVfsHttpError {
                status: None,
                code: None,
                message: err.to_string(),
            })?;

        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| DbVfsHttpError {
                status: Some(status),
                code: None,
                message: err.to_string(),
            })?
            .to_vec();

        if status.is_success() {
            return serde_json::from_slice(&body).map_err(|err| DbVfsHttpError {
                status: Some(status),
                code: None,
                message: format!("invalid json: {err}"),
            });
        }

        if let Ok(err) = serde_json::from_slice::<DbVfsErrorBody>(&body) {
            return Err(DbVfsHttpError {
                status: Some(status),
                code: Some(err.code),
                message: err.message,
            });
        }

        Err(DbVfsHttpError {
            status: Some(status),
            code: None,
            message: String::from_utf8_lossy(&body).to_string(),
        })
    }
}

#[derive(Debug, Clone)]
struct DbVfsHttpError {
    status: Option<reqwest::StatusCode>,
    code: Option<String>,
    message: String,
}

impl DbVfsHttpError {
    fn is_denied(&self) -> bool {
        matches!(
            self.code.as_deref(),
            Some("not_permitted") | Some("secret_path_denied")
        )
    }
}

impl std::fmt::Display for DbVfsHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.status, &self.code) {
            (Some(status), Some(code)) => write!(f, "db-vfs error ({status}, {code}): {}", self.message),
            (Some(status), None) => write!(f, "db-vfs error ({status}): {}", self.message),
            (None, Some(code)) => write!(f, "db-vfs error ({code}): {}", self.message),
            (None, None) => write!(f, "db-vfs error: {}", self.message),
        }
    }
}

impl std::error::Error for DbVfsHttpError {}

#[derive(Debug, Deserialize)]
struct DbVfsErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct DbVfsReadRequest {
    workspace_id: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_line: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DbVfsReadResponse {
    #[allow(dead_code)]
    path: String,
    #[allow(dead_code)]
    bytes_read: u64,
    content: String,
    #[allow(dead_code)]
    truncated: bool,
    #[allow(dead_code)]
    #[serde(default)]
    start_line: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    end_line: Option<u64>,
    version: u64,
}

#[derive(Debug, Serialize)]
struct DbVfsWriteRequest {
    workspace_id: String,
    path: String,
    content: String,
    expected_version: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DbVfsWriteResponse {
    #[allow(dead_code)]
    path: String,
    #[allow(dead_code)]
    bytes_written: u64,
    #[allow(dead_code)]
    created: bool,
    #[allow(dead_code)]
    version: u64,
}

#[derive(Debug, Serialize)]
struct DbVfsDeleteRequest {
    workspace_id: String,
    path: String,
    expected_version: Option<u64>,
    ignore_missing: bool,
}

#[derive(Debug, Deserialize)]
struct DbVfsDeleteResponse {
    #[allow(dead_code)]
    path: String,
    deleted: bool,
}

#[derive(Debug, Serialize)]
struct DbVfsGlobRequest {
    workspace_id: String,
    pattern: String,
    path_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DbVfsGlobResponse {
    matches: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct DbVfsGrepRequest {
    workspace_id: String,
    query: String,
    regex: bool,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    path_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DbVfsGrepResponse {
    matches: Vec<DbVfsGrepMatch>,
    truncated: bool,
    #[serde(default)]
    skipped_too_large_files: u64,
    #[serde(default)]
    scanned_files: u64,
}

#[derive(Debug, Deserialize)]
struct DbVfsGrepMatch {
    path: String,
    line: u64,
    text: String,
}
