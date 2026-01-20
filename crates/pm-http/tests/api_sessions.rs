use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_core::{FsStorage, PmPaths, SessionId, Storage};
use tower::ServiceExt;

#[tokio::test]
async fn get_session_rejects_invalid_session_id() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let app = pm_http::router(pm_paths)?;

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
    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let app = pm_http::router(pm_paths)?;

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
    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));

    let id = SessionId::new();
    let storage = FsStorage::new(pm_paths.data_dir());
    storage
        .put_json(
            &format!("sessions/{id}/session"),
            &serde_json::json!({"id": id, "ok": true}),
        )
        .await?;

    let app = pm_http::router(pm_paths)?;
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

