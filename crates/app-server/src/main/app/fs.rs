use super::*;

pub(super) async fn handle_fs_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "fs/mkdir" => FsMkdirParams => |params| handle_fs_mkdir(server, params),
    })
}
