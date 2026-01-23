#[cfg(test)]
mod mcp_tests {
    use super::*;

    #[tokio::test]
    async fn load_mcp_config_defaults_to_empty_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_mcp_config(dir.path()).await.unwrap();
        assert!(cfg.path.is_none());
        assert!(cfg.servers.is_empty());
    }

    #[tokio::test]
    async fn load_mcp_config_parses_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let spec_dir = dir.path().join(".codepm_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await.unwrap();
        tokio::fs::write(
            spec_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "rg": { "transport": "stdio", "argv": ["mcp-rg", "--stdio"], "env": { "NO_COLOR": "1" } } } }"#,
        )
        .await
        .unwrap();

        let cfg = load_mcp_config(dir.path()).await.unwrap();
        assert!(cfg.path.is_some());
        assert_eq!(cfg.servers.len(), 1);
        let server = cfg.servers.get("rg").unwrap();
        assert_eq!(server.argv, vec!["mcp-rg".to_string(), "--stdio".to_string()]);
        assert!(server.env.contains_key("NO_COLOR"));
    }

    #[tokio::test]
    async fn load_mcp_config_denies_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let spec_dir = dir.path().join(".codepm_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await.unwrap();
        tokio::fs::write(
            spec_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": {}, "extra": 123 }"#,
        )
        .await
        .unwrap();

        let err = load_mcp_config(dir.path()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("parse"), "err={msg}");
    }

    #[tokio::test]
    async fn load_mcp_config_denies_invalid_server_names() {
        let dir = tempfile::tempdir().unwrap();
        let spec_dir = dir.path().join(".codepm_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await.unwrap();
        tokio::fs::write(
            spec_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "bad name": { "transport": "stdio", "argv": ["x"] } } }"#,
        )
        .await
        .unwrap();

        let err = load_mcp_config(dir.path()).await.unwrap_err();
        assert!(err.to_string().contains("invalid mcp server name"));
    }

    #[tokio::test]
    async fn load_mcp_config_env_path_is_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_mcp_config_inner(dir.path(), Some(PathBuf::from("missing.json")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("read"));
    }
}
