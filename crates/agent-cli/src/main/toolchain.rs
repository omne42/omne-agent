async fn run_toolchain_bootstrap(args: ToolchainBootstrapArgs) -> anyhow::Result<()> {
    let request = omne_toolchain_runtime::ToolchainBootstrapRequest {
        target_triple: args.target_triple.clone(),
        bundled_dir: args.bundled_dir.clone(),
        managed_dir: args.managed_dir.clone(),
    };
    let result = omne_toolchain_runtime::bootstrap_toolchain(&request).await?;

    if args.json {
        let value = serde_json::to_value(&result).context("serialize toolchain/bootstrap response")?;
        print_json_or_pretty(true, &value)?;
    } else {
        println!(
            "toolchain bootstrap: target={} managed_dir={} bundled_dir={}",
            result.target_triple,
            result.managed_dir,
            result.bundled_dir.as_deref().unwrap_or("-")
        );
        for item in &result.items {
            let detail = item
                .detail
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            println!("- {}: {}{}", item.tool, item.status.as_str(), detail);
        }
    }

    if args.strict && omne_toolchain_runtime::has_bootstrap_failure(&result.items) {
        anyhow::bail!("toolchain bootstrap did not satisfy strict mode requirements");
    }
    Ok(())
}
