fn resolve_reasoning_summary_text(mut summary: String, output_items: &[Value]) -> String {
    if !summary.trim().is_empty() {
        return summary;
    }

    let mut saw_reasoning_item = false;
    let mut saw_encrypted_content = false;
    let mut summary_from_items: Option<String> = None;

    for item in output_items {
        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
            continue;
        }
        saw_reasoning_item = true;
        if item.get("encrypted_content").is_some() {
            saw_encrypted_content = true;
        }
        match item.get("summary") {
            Some(Value::Array(parts)) => {
                let mut text = String::new();
                for part in parts {
                    match part {
                        Value::String(s) => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(s);
                        }
                        Value::Object(obj) => {
                            if let Some(s) = obj.get("text").and_then(Value::as_str) {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(s);
                            }
                        }
                        _ => {}
                    }
                }
                if !text.trim().is_empty() {
                    summary_from_items = Some(text);
                    break;
                }
            }
            Some(Value::String(s)) => {
                if !s.trim().is_empty() {
                    summary_from_items = Some(s.clone());
                    break;
                }
            }
            _ => {}
        }
    }

    summary = match summary_from_items {
        Some(text) => text,
        None if saw_reasoning_item && saw_encrypted_content => {
            "(reasoning was returned as encrypted_content; summary unavailable)".to_string()
        }
        None => String::new(),
    };

    summary
}

