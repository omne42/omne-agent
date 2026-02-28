use super::*;

pub(super) async fn handle_repo_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "repo/search" => RepoSearchParams => |params| handle_repo_search(server, params),
        "repo/index" => RepoIndexParams => |params| handle_repo_index(server, params),
        "repo/symbols" => RepoSymbolsParams => |params| handle_repo_symbols(server, params),
    })
}
