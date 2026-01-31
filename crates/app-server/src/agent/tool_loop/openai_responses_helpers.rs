fn content_part_to_openai_user_item(part: &ditto_llm::ContentPart) -> Option<Value> {
    match part {
        ditto_llm::ContentPart::Text { text } => {
            if text.is_empty() {
                return None;
            }
            Some(serde_json::json!({ "type": "input_text", "text": text }))
        }
        ditto_llm::ContentPart::Image { source } => {
            let image_url = match source {
                ditto_llm::ImageSource::Url { url } => url.clone(),
                ditto_llm::ImageSource::Base64 { media_type, data } => {
                    format!("data:{media_type};base64,{data}")
                }
            };
            Some(serde_json::json!({ "type": "input_image", "image_url": image_url }))
        }
        ditto_llm::ContentPart::File {
            filename,
            media_type,
            source,
        } => {
            if media_type != "application/pdf" {
                return None;
            }

            let item = match source {
                ditto_llm::FileSource::Url { url } => {
                    serde_json::json!({ "type": "input_file", "file_url": url })
                }
                ditto_llm::FileSource::Base64 { data } => serde_json::json!({
                    "type": "input_file",
                    "filename": filename.clone().unwrap_or_else(|| "file.pdf".to_string()),
                    "file_data": format!("data:{media_type};base64,{data}"),
                }),
                ditto_llm::FileSource::FileId { file_id } => {
                    serde_json::json!({ "type": "input_file", "file_id": file_id })
                }
            };
            Some(item)
        }
        _ => None,
    }
}

fn build_user_message_item(text: &str, attachment_parts: &[ditto_llm::ContentPart]) -> Option<Value> {
    let mut content = Vec::<Value>::new();
    if !text.trim().is_empty() {
        content.push(serde_json::json!({ "type": "input_text", "text": text }));
    }
    for part in attachment_parts {
        if let Some(item) = content_part_to_openai_user_item(part) {
            content.push(item);
        }
    }
    if content.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "type": "message",
        "role": "user",
        "content": content,
    }))
}

fn append_attachments_to_last_user_message(
    history: &mut [Value],
    attachment_parts: &[ditto_llm::ContentPart],
) -> bool {
    if attachment_parts.is_empty() {
        return false;
    }

    let Some(last_user_idx) = history.iter().rposition(|item| {
        item.get("type").and_then(Value::as_str) == Some("message")
            && item.get("role").and_then(Value::as_str) == Some("user")
    }) else {
        return false;
    };

    let Some(obj) = history[last_user_idx].as_object_mut() else {
        return false;
    };
    let Some(content) = obj.get_mut("content").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut added = false;
    for part in attachment_parts {
        if let Some(item) = content_part_to_openai_user_item(part) {
            content.push(item);
            added = true;
        }
    }
    added
}

fn parse_function_call_item(item: &Value) -> Option<(String, String, String)> {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let call_id = item.get("call_id").and_then(Value::as_str)?;
    let name = item.get("name").and_then(Value::as_str)?;
    let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("{}");
    Some((name.to_string(), arguments.to_string(), call_id.to_string()))
}

