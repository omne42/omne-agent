use super::*;

pub(super) async fn handle_approval_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "approval/decide" => ApprovalDecideParams => |params| handle_approval_decide(server, params),
        "approval/list" => ApprovalListParams => |params| handle_approval_list(server, params),
    })
}
