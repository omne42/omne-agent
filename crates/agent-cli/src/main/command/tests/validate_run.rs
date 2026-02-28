use super::*;

#[tokio::test]
async fn run_command_validate_accepts_valid_files() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-ok");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("plan.md"),
        r#"---
version: 1
mode: architect
---
Plan body
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = run_command_validate(&cli, None, false, true).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    result
}

#[tokio::test]
async fn collect_command_validate_result_sets_all_target_when_name_absent() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-target-all");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("plan.md"),
        r#"---
version: 1
mode: architect
---
Plan body
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, None, false).await?;
    assert_eq!(result.target, "all");
    assert_eq!(result.summary.commands_dir, commands_dir.display().to_string());
    assert!(result.summary.ok);
    assert_eq!(result.summary.item_count, 1);
    assert_eq!(result.validated_count, 1);
    assert_eq!(result.summary.error_count, 0);
    assert!(result.errors.is_empty());
    assert_eq!(result.validated.len(), 1);

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn collect_command_validate_result_sets_named_target() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-target-name");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("plan.md"),
        r#"---
version: 1
mode: architect
---
Plan body
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let result = collect_command_validate_result(&cli, Some("plan".to_string()), false).await?;
    assert_eq!(result.target, "plan");
    assert_eq!(result.summary.commands_dir, commands_dir.display().to_string());
    assert!(result.summary.ok);
    assert_eq!(result.summary.item_count, 1);
    assert_eq!(result.validated_count, 1);
    assert_eq!(result.summary.error_count, 0);
    assert!(result.errors.is_empty());
    assert_eq!(result.validated.len(), 1);

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn run_command_validate_fails_on_invalid_frontmatter() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-invalid");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("broken.md"),
        r#"---
version: 1
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

    let err = run_command_validate(&cli, None, false, true)
        .await
        .expect_err("invalid command should fail validation");
    assert!(err.to_string().contains("command validation failed"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn load_workflow_file_exposes_modes_load_error_when_modes_config_parse_fails()
-> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-load-workflow-modes-load-error");
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

    let wf = load_workflow_file(&cli, "ok").await?;
    assert_eq!(wf.frontmatter.mode, "coder");
    assert!(
        wf.modes_load_error
            .as_deref()
            .is_some_and(|msg| msg.contains("parse modes config"))
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn run_command_validate_strict_rejects_duplicate_declared_names() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-strict");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("a.md"),
        r#"---
version: 1
name: duplicate
mode: reviewer
---
A
"#,
    )
    .await?;
    tokio::fs::write(
        commands_dir.join("b.md"),
        r#"---
version: 1
name: duplicate
mode: reviewer
---
B
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    run_command_validate(&cli, None, false, true).await?;
    let err = run_command_validate(&cli, None, true, true)
        .await
        .expect_err("strict validation should reject duplicate names");
    assert!(err.to_string().contains("command validation failed"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn run_command_validate_named_target_reports_missing_file() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-missing");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let err = run_command_validate(&cli, Some("missing".to_string()), false, true)
        .await
        .expect_err("missing named command should fail");
    let msg = err.to_string();
    assert!(msg.contains("command `missing` validation failed"));
    assert!(msg.contains("No such file or directory"));

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn run_command_validate_named_target_rejects_invalid_name() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-invalid-name");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let err = run_command_validate(&cli, Some("../bad".to_string()), false, true)
        .await
        .expect_err("invalid workflow name should be rejected");
    assert!(err.to_string().contains("workflow name must not contain path separators"));

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn run_command_validate_named_target_reports_parse_error_with_name() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-named-parse");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;
    tokio::fs::write(
        commands_dir.join("broken.md"),
        r#"---
version: 1
---
broken
"#,
    )
    .await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    let err = run_command_validate(&cli, Some("broken".to_string()), false, true)
        .await
        .expect_err("invalid named command should fail");
    let msg = err.to_string();
    assert!(msg.contains("command `broken` validation failed"));
    assert!(msg.contains("missing field `mode`"));

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}

#[tokio::test]
async fn run_command_validate_allows_empty_command_dir() -> anyhow::Result<()> {
    let tmp = unique_temp_dir("omne-command-validate-empty-dir");
    let commands_dir = tmp.join("spec").join("commands");
    tokio::fs::create_dir_all(&commands_dir).await?;

    let cli = Cli {
        omne_root: Some(tmp.clone()),
        app_server: None,
        execpolicy_rules: vec![],
        command: None,
    };

    run_command_validate(&cli, None, false, true).await?;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}
