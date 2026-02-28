use super::*;

pub(super) async fn handle_mcp_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "mcp/list_servers" => McpListServersParams => |params| handle_mcp_list_servers(server, params),
        "mcp/list_tools" => McpListToolsParams => |params| handle_mcp_list_tools(server, params),
        "mcp/list_resources" => McpListResourcesParams => |params| handle_mcp_list_resources(server, params),
        "mcp/call" => McpCallParams => |params| handle_mcp_call(server, params),
    })
}
