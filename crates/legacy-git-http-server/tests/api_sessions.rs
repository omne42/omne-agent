use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use omne_core::domain::SessionMeta;
use omne_core::{FsStorage, PmPaths, SessionId, Storage};
use tower::ServiceExt;

#[tokio::test]
async fn get_session_rejects_invalid_session_id() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));
    let app = omne_http::router(omne_paths)?;

    let request = Request::builder()
        .uri("/api/v0/sessions/not-a-uuid/session")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn get_session_returns_404_when_missing() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));
    let app = omne_http::router(omne_paths)?;

    let id = SessionId::new();
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}/session"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.into_body().collect().await?.to_bytes();
    assert_eq!(std::str::from_utf8(&body)?, "session not found");
    Ok(())
}

#[tokio::test]
async fn get_session_returns_json_when_present() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id = SessionId::new();
    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id}/session"),
            &serde_json::json!({"id": id, "ok": true}),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}/session"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(value["id"], serde_json::json!(id));
    assert_eq!(value["ok"], serde_json::Value::Bool(true));
    Ok(())
}

#[tokio::test]
async fn get_session_bundle_prefers_result_by_default() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id = SessionId::new();
    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id}/session"),
            &serde_json::json!({"id": id, "stage": "session"}),
        )
        .await?;
    storage
        .put_json(
            &format!("sessions/{id}/result"),
            &serde_json::json!({"id": id, "stage": "result"}),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(value["result"]["stage"], "result");
    assert!(value.get("session").is_none());
    Ok(())
}

#[tokio::test]
async fn get_session_bundle_all_includes_all_present_keys() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id = SessionId::new();
    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id}/session"),
            &serde_json::json!({"id": id}),
        )
        .await?;
    storage
        .put_json(&format!("sessions/{id}/tasks"), &serde_json::json!([]))
        .await?;
    storage
        .put_json(&format!("sessions/{id}/prs"), &serde_json::json!([]))
        .await?;
    storage
        .put_json(
            &format!("sessions/{id}/merge"),
            &serde_json::json!({"merged": true}),
        )
        .await?;
    storage
        .put_json(
            &format!("sessions/{id}/result"),
            &serde_json::json!({"id": id}),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}?all=true"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    for key in ["session", "tasks", "prs", "merge", "result"] {
        assert!(value.get(key).is_some(), "missing key {key}");
    }
    Ok(())
}

#[tokio::test]
async fn get_session_meta_returns_404_when_missing() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));
    let app = omne_http::router(omne_paths)?;

    let id = SessionId::new();
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}/meta"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn get_session_meta_returns_json_when_present() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id = SessionId::new();
    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id}/session"),
            &serde_json::json!({
                "id": id,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "big prompt",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:00Z",
            }),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}/meta"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(value["id"], serde_json::json!(id));
    assert!(value.get("prompt").is_none());
    Ok(())
}

#[tokio::test]
async fn get_session_bundle_all_flag_without_value_includes_all_present_keys() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id = SessionId::new();
    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id}/session"),
            &serde_json::json!({"id": id}),
        )
        .await?;
    storage
        .put_json(&format!("sessions/{id}/tasks"), &serde_json::json!([]))
        .await?;
    storage
        .put_json(&format!("sessions/{id}/prs"), &serde_json::json!([]))
        .await?;
    storage
        .put_json(
            &format!("sessions/{id}/merge"),
            &serde_json::json!({"merged": true}),
        )
        .await?;
    storage
        .put_json(
            &format!("sessions/{id}/result"),
            &serde_json::json!({"id": id}),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri(format!("/api/v0/sessions/{id}?all"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    for key in ["session", "tasks", "prs", "merge", "result"] {
        assert!(value.get(key).is_some(), "missing key {key}");
    }
    Ok(())
}

#[tokio::test]
async fn list_sessions_returns_session_ids() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
    let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(&format!("sessions/{id2}/tasks"), &serde_json::json!([]))
        .await?;
    storage
        .put_json(
            &format!("sessions/{id1}/session"),
            &serde_json::json!({"id": id1}),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri("/api/v0/sessions")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let ids: Vec<SessionId> = serde_json::from_slice(&bytes)?;
    assert_eq!(ids, vec![id1, id2]);
    Ok(())
}

#[tokio::test]
async fn list_sessions_limit_truncates() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
    let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(&format!("sessions/{id2}/tasks"), &serde_json::json!([]))
        .await?;
    storage
        .put_json(
            &format!("sessions/{id1}/session"),
            &serde_json::json!({"id": id1}),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri("/api/v0/sessions?limit=1")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let ids: Vec<SessionId> = serde_json::from_slice(&bytes)?;
    assert_eq!(ids, vec![id1]);
    Ok(())
}

#[tokio::test]
async fn list_sessions_verbose_returns_session_meta_sorted() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
    let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id1}/session"),
            &serde_json::json!({
                "id": id1,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "old",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:10Z",
            }),
        )
        .await?;
    storage
        .put_json(
            &format!("sessions/{id2}/session"),
            &serde_json::json!({
                "id": id2,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "new",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:20Z",
            }),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri("/api/v0/sessions?verbose=true")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", std::str::from_utf8(&bytes)?);
    let sessions: Vec<SessionMeta> = serde_json::from_slice(&bytes)?;
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, id2);
    assert_eq!(sessions[1].id, id1);
    Ok(())
}

#[tokio::test]
async fn list_sessions_verbose_flag_without_value_is_true() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
    let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id1}/session"),
            &serde_json::json!({
                "id": id1,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "old",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:10Z",
            }),
        )
        .await?;
    storage
        .put_json(
            &format!("sessions/{id2}/session"),
            &serde_json::json!({
                "id": id2,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "new",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:20Z",
            }),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri("/api/v0/sessions?verbose")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", std::str::from_utf8(&bytes)?);

    let sessions: Vec<SessionMeta> = serde_json::from_slice(&bytes)?;
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, id2);
    assert_eq!(sessions[1].id, id1);
    Ok(())
}

#[tokio::test]
async fn list_sessions_verbose_limit_truncates() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(tmp.path().join(".omne"));

    let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
    let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

    let storage = FsStorage::new(omne_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id1}/session"),
            &serde_json::json!({
                "id": id1,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "old",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:10Z",
            }),
        )
        .await?;
    storage
        .put_json(
            &format!("sessions/{id2}/session"),
            &serde_json::json!({
                "id": id2,
                "repo": "repo",
                "pr_name": "pr",
                "prompt": "new",
                "base_branch": "main",
                "created_at": "2026-01-20T00:00:20Z",
            }),
        )
        .await?;

    let app = omne_http::router(omne_paths)?;
    let request = Request::builder()
        .uri("/api/v0/sessions?verbose=true&limit=1")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", std::str::from_utf8(&bytes)?);
    let sessions: Vec<SessionMeta> = serde_json::from_slice(&bytes)?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, id2);
    Ok(())
}
