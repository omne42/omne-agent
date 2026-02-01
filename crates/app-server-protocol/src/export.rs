use std::fs;
use std::path::Path;

use anyhow::Context;
use schemars::JsonSchema;
use schemars::schema_for;
use ts_rs::TS;

use crate::{
    ClientRequest, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse, RequestId,
    ServerNotification,
};

pub fn generate_ts(out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create out dir {}", out_dir.display()))?;

    RequestId::export_all_to(out_dir).context("export RequestId typescript")?;
    JsonRpcRequest::export_all_to(out_dir).context("export JsonRpcRequest typescript")?;
    JsonRpcResponse::export_all_to(out_dir).context("export JsonRpcResponse typescript")?;
    JsonRpcErrorResponse::export_all_to(out_dir)
        .context("export JsonRpcErrorResponse typescript")?;
    JsonRpcError::export_all_to(out_dir).context("export JsonRpcError typescript")?;
    ClientRequest::export_all_to(out_dir).context("export ClientRequest typescript")?;
    ServerNotification::export_all_to(out_dir).context("export ServerNotification typescript")?;
    omne_agent_protocol::ThreadEvent::export_all_to(out_dir)
        .context("export ThreadEvent typescript")?;

    Ok(())
}

pub fn generate_json_schema(out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create out dir {}", out_dir.display()))?;

    write_schema::<RequestId>(out_dir, "RequestId")?;
    write_schema::<JsonRpcRequest>(out_dir, "JsonRpcRequest")?;
    write_schema::<JsonRpcResponse>(out_dir, "JsonRpcResponse")?;
    write_schema::<JsonRpcErrorResponse>(out_dir, "JsonRpcErrorResponse")?;
    write_schema::<JsonRpcError>(out_dir, "JsonRpcError")?;
    write_schema::<ClientRequest>(out_dir, "ClientRequest")?;
    write_schema::<ServerNotification>(out_dir, "ServerNotification")?;
    write_schema::<omne_agent_protocol::ThreadEvent>(out_dir, "ThreadEvent")?;

    Ok(())
}

fn write_schema<T>(out_dir: &Path, name: &str) -> anyhow::Result<()>
where
    T: JsonSchema,
{
    let schema = schema_for!(T);
    let contents = serde_json::to_string_pretty(&schema)?;
    fs::write(out_dir.join(format!("{name}.schema.json")), contents)
        .with_context(|| format!("write schema {name}"))?;
    Ok(())
}
