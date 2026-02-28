use super::*;

#[tokio::test]
async fn collect_command_validate_result_reports_unknown_allowed_tool() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-unknown-tool");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("broken.md"),
        r#"---
version: 1
mode: coder
allowed_tools:
  - process/start
  - tool/does_not_exist
---
Broken
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
    assert!(!result.summary.ok);
    assert_eq!(result.summary.error_count, 1);
    assert!(
        result.errors[0]
            .error
            .contains("unknown tool in allowed_tools: tool/does_not_exist")
    );
    assert_eq!(
        result.errors[0].error_code.as_deref(),
        Some("allowed_tools_unknown_tool")
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn collect_command_validate_result_reports_mode_incompatible_allowed_tool() -> anyhow::Result<()>
{
    let tmp = unique_temp_dir("omne-command-validate-mode-denied-tool");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("broken.md"),
        r#"---
version: 1
mode: reviewer
allowed_tools:
  - file/write
---
Broken
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
    assert!(!result.summary.ok);
    assert_eq!(result.summary.error_count, 1);
    let err_text = &result.errors[0].error;
    assert!(err_text.contains("allowed_tools tool is denied by mode"));
    assert!(err_text.contains("file/write"));
    assert_eq!(
        result.errors[0].error_code.as_deref(),
        Some("allowed_tools_mode_denied")
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn collect_command_validate_result_reports_unknown_mode() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-unknown-mode");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("broken.md"),
        r#"---
version: 1
mode: mode-does-not-exist
---
Broken
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
    assert!(!result.summary.ok);
    assert_eq!(result.summary.error_count, 1);
    assert!(
        result.errors[0]
            .error
            .contains("unknown mode: mode-does-not-exist")
    );
    assert_eq!(result.errors[0].error_code.as_deref(), Some("mode_unknown"));

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn collect_command_validate_result_respects_custom_mode_tool_override_denies()
-> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-custom-mode-deny");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::create_dir_all(tmp.join(".omne_data").join("spec")).await?;
    tokio::fs::write(
        tmp.join(".omne_data").join("spec").join("modes.yaml"),
        r#"version: 1
modes:
  strict-coder:
    description: "custom mode with process/start denied"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
      command:
        decision: allow
      process:
        inspect: { decision: allow }
        kill: { decision: allow }
        interact: { decision: deny }
      artifact: { decision: allow }
      browser: { decision: deny }
    tool_overrides:
      - tool: "process/start"
        decision: deny
"#,
    )
    .await?;
    tokio::fs::write(
        commands_dir.join("broken.md"),
        r#"---
version: 1
mode: strict-coder
allowed_tools:
  - process/start
---
Broken
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
    assert!(!result.summary.ok);
    assert_eq!(result.summary.error_count, 1);
    let err_text = &result.errors[0].error;
    assert!(err_text.contains("allowed_tools tool is denied by mode"));
    assert!(err_text.contains("process/start"));
    assert_eq!(
        result.errors[0].error_code.as_deref(),
        Some("allowed_tools_mode_denied")
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn collect_command_validate_result_exposes_modes_load_error_for_json_clients()
-> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-modes-load-error");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::create_dir_all(tmp.join(".omne_data").join("spec")).await?;
    tokio::fs::write(
        tmp.join(".omne_data").join("spec").join("modes.yaml"),
        "version: [not-a-number]\n",
    )
    .await?;
    tokio::fs::write(
        commands_dir.join("ok.md"),
        r#"---
version: 1
mode: coder
---
Ok
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, Some("ok".to_string()), false).await?;
    assert!(result.summary.ok);
    assert_eq!(result.summary.error_count, 0);
    assert!(result.modes_load_error.is_some());
    assert!(
        result
            .modes_load_error
            .as_deref()
            .is_some_and(|msg| msg.contains("parse modes config"))
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}
