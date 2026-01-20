use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_core::PmPaths;
use tower::ServiceExt;

#[tokio::test]
async fn list_repos_returns_repo_names() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));

    tokio::fs::create_dir_all(pm_paths.repos_dir().join("demo.git")).await?;

    let app = pm_http::router(pm_paths.clone())?;
    let request = Request::builder()
        .uri("/api/v0/repos")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(value, serde_json::json!(["demo"]));
    Ok(())
}

#[tokio::test]
async fn list_repos_verbose_returns_repo_metadata() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));

    tokio::fs::create_dir_all(pm_paths.repos_dir().join("demo.git")).await?;

    let app = pm_http::router(pm_paths.clone())?;
    let request = Request::builder()
        .uri("/api/v0/repos?verbose=true")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let items = value.as_array().expect("expected array");
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["name"], "demo");
    assert_eq!(
        item["bare_path"],
        pm_paths.repos_dir().join("demo.git").display().to_string()
    );
    assert_eq!(
        item["lock_path"],
        pm_paths.locks_dir().join("demo.lock").display().to_string()
    );
    Ok(())
}

#[tokio::test]
async fn list_repos_verbose_flag_without_value_is_true() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));

    tokio::fs::create_dir_all(pm_paths.repos_dir().join("demo.git")).await?;

    let app = pm_http::router(pm_paths.clone())?;
    let request = Request::builder()
        .uri("/api/v0/repos?verbose")
        .body(Body::empty())?;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let items = value.as_array().expect("expected array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "demo");
    Ok(())
}
