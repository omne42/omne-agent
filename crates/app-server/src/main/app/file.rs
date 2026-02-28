use super::*;

pub(super) async fn handle_file_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "file/read" => FileReadParams => |params| handle_file_read(server, params),
        "file/glob" => FileGlobParams => |params| handle_file_glob(server, params),
        "file/grep" => FileGrepParams => |params| handle_file_grep(server, params),
        "file/write" => FileWriteParams => |params| handle_file_write(server, params),
        "file/patch" => FilePatchParams => |params| handle_file_patch(server, params),
        "file/edit" => FileEditParams => |params| handle_file_edit(server, params),
        "file/delete" => FileDeleteParams => |params| handle_file_delete(server, params),
    })
}
