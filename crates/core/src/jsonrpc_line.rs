use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcLine {
    Request(JsonRpcLineRequest),
    Notification(JsonRpcLineNotification),
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcLineRequest {
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcLineNotification {
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcLineParseError {
    pub id: Value,
    pub code: i64,
    pub message: &'static str,
    pub data: Option<String>,
}

pub fn parse_jsonrpc_line(line: &str) -> Result<Option<JsonRpcLine>, JsonRpcLineParseError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }

    let msg: Value = serde_json::from_str(line).map_err(|err| JsonRpcLineParseError {
        id: Value::Null,
        code: -32700,
        message: "parse error",
        data: Some(err.to_string()),
    })?;
    let Some(obj) = msg.as_object() else {
        return Err(JsonRpcLineParseError {
            id: Value::Null,
            code: -32600,
            message: "invalid request",
            data: None,
        });
    };

    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Err(JsonRpcLineParseError {
            id: Value::Null,
            code: -32600,
            message: "invalid request",
            data: None,
        });
    };
    let method = method.to_string();
    let id = obj.get("id").cloned();
    let params = obj.get("params").cloned();

    match id {
        None => Ok(Some(JsonRpcLine::Notification(JsonRpcLineNotification {
            method,
            params,
        }))),
        Some(id) if id.is_null() => Ok(None),
        Some(id) => Ok(Some(JsonRpcLine::Request(JsonRpcLineRequest {
            id,
            method,
            params,
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_empty_lines() {
        assert_eq!(parse_jsonrpc_line(""), Ok(None));
        assert_eq!(parse_jsonrpc_line("  "), Ok(None));
    }

    #[test]
    fn parse_returns_request() {
        let parsed = parse_jsonrpc_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .expect("parse should succeed");
        assert_eq!(
            parsed,
            Some(JsonRpcLine::Request(JsonRpcLineRequest {
                id: Value::from(1),
                method: "tools/list".to_string(),
                params: None,
            }))
        );
    }

    #[test]
    fn parse_returns_notification() {
        let parsed =
            parse_jsonrpc_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .expect("parse should succeed");
        assert_eq!(
            parsed,
            Some(JsonRpcLine::Notification(JsonRpcLineNotification {
                method: "notifications/initialized".to_string(),
                params: None,
            }))
        );
    }

    #[test]
    fn parse_skips_null_id_requests() {
        let parsed = parse_jsonrpc_line(r#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#)
            .expect("parse should succeed");
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_returns_parse_error() {
        let err = parse_jsonrpc_line("{").expect_err("parse error should be returned");
        assert_eq!(err.code, -32700);
        assert_eq!(err.message, "parse error");
        assert_eq!(err.id, Value::Null);
        assert!(err.data.is_some());
    }

    #[test]
    fn parse_returns_invalid_request_error() {
        let err = parse_jsonrpc_line("[]").expect_err("invalid request should be returned");
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "invalid request");
        assert_eq!(err.id, Value::Null);
        assert_eq!(err.data, None);
    }
}
