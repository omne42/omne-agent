use super::*;

pub(super) async fn handle_artifact_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "artifact/write" => ArtifactWriteParams => |params| handle_artifact_write(server, params),
        "artifact/list" => ArtifactListParams => |params| handle_artifact_list(server, params),
        "artifact/read" => ArtifactReadParams => |params| handle_artifact_read(server, params),
        "artifact/versions" => ArtifactVersionsParams => |params| handle_artifact_versions(server, params),
        "artifact/delete" => ArtifactDeleteParams => |params| handle_artifact_delete(server, params),
    })
}
