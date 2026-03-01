struct App {
    rpc: omne_jsonrpc::Client,
    notifications: Option<tokio::sync::mpsc::Receiver<omne_jsonrpc::Notification>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum McpListServersOrFailedResponse {
    List(omne_app_server_protocol::McpListServersResponse),
    Failed(omne_app_server_protocol::McpFailedResponse),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum McpActionOrFailedResponse {
    Action(omne_app_server_protocol::McpActionResponse),
    Failed(omne_app_server_protocol::McpFailedResponse),
}

type ThreadConfigExplainResponse = omne_app_server_protocol::ThreadConfigExplainResponse;

type ParsedTurnInput = (
    String,
    Vec<omne_protocol::ContextRef>,
    Vec<omne_protocol::TurnAttachment>,
    Vec<omne_protocol::TurnDirective>,
);

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ThreadSpawnResponse {
    thread_id: ThreadId,
    turn_id: TurnId,
    log_path: String,
    last_seq: u64,
}

fn split_special_directives(
    input: &str,
) -> anyhow::Result<ParsedTurnInput> {
    let mut refs = Vec::<omne_protocol::ContextRef>::new();
    let mut attachments = Vec::<omne_protocol::TurnAttachment>::new();
    let mut directives = Vec::<omne_protocol::TurnDirective>::new();
    let lines = input.lines().collect::<Vec<_>>();

    let mut idx = 0usize;
    let mut did_parse = false;
    while idx < lines.len() {
        let line = lines[idx];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            idx += 1;
            continue;
        }

        if trimmed == "@file" {
            anyhow::bail!("@file requires a path");
        }
        if trimmed.starts_with("@file ") || trimmed.starts_with("@file\t") {
            let spec = trimmed["@file".len()..].trim();
            let (path, start_line, end_line) = parse_file_ref_spec(spec)?;
            refs.push(omne_protocol::ContextRef::File(omne_protocol::ContextRefFile {
                path,
                start_line,
                end_line,
                max_bytes: None,
            }));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed.starts_with("@diff") && trimmed != "@diff" {
            anyhow::bail!("@diff does not accept arguments");
        }
        if trimmed == "@diff" {
            refs.push(omne_protocol::ContextRef::Diff(omne_protocol::ContextRefDiff { max_bytes: None }));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed == "@image" {
            anyhow::bail!("@image requires a path or url");
        }
        if trimmed.starts_with("@image ") || trimmed.starts_with("@image\t") {
            let spec = trimmed["@image".len()..].trim();
            let source = if spec.starts_with("http://") || spec.starts_with("https://") {
                omne_protocol::AttachmentSource::Url {
                    url: spec.to_string(),
                }
            } else {
                omne_protocol::AttachmentSource::Path {
                    path: spec.to_string(),
                }
            };
            attachments.push(omne_protocol::TurnAttachment::Image(
                omne_protocol::TurnAttachmentImage {
                    source,
                    media_type: None,
                },
            ));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed == "@pdf" {
            anyhow::bail!("@pdf requires a path or url");
        }
        if trimmed.starts_with("@pdf ") || trimmed.starts_with("@pdf\t") {
            let spec = trimmed["@pdf".len()..].trim();
            let source = if spec.starts_with("http://") || spec.starts_with("https://") {
                omne_protocol::AttachmentSource::Url {
                    url: spec.to_string(),
                }
            } else {
                omne_protocol::AttachmentSource::Path {
                    path: spec.to_string(),
                }
            };
            attachments.push(omne_protocol::TurnAttachment::File(
                omne_protocol::TurnAttachmentFile {
                    source,
                    media_type: "application/pdf".to_string(),
                    filename: None,
                },
            ));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed.starts_with("/plan") && trimmed != "/plan" {
            anyhow::bail!("/plan does not accept arguments");
        }
        if trimmed == "/plan" {
            if directives
                .iter()
                .any(|directive| matches!(directive, omne_protocol::TurnDirective::Plan))
            {
                anyhow::bail!("duplicate directive: /plan");
            }
            directives.push(omne_protocol::TurnDirective::Plan);
            did_parse = true;
            idx += 1;
            continue;
        }

        break;
    }

    if !did_parse {
        return Ok((input.to_string(), refs, attachments, directives));
    }

    Ok((lines[idx..].join("\n"), refs, attachments, directives))
}

fn parse_file_ref_spec(spec: &str) -> anyhow::Result<(String, Option<u64>, Option<u64>)> {
    let spec = spec.trim();
    if spec.is_empty() {
        anyhow::bail!("file ref is empty");
    }

    let mut parts = spec.split(':').collect::<Vec<_>>();
    let last = parts.pop().unwrap_or_default().trim();
    let Ok(last_num) = last.parse::<u64>() else {
        return Ok((spec.to_string(), None, None));
    };

    if last_num == 0 {
        anyhow::bail!("line numbers must be >= 1");
    }

    let prev = parts.last().copied().unwrap_or_default().trim();
    let prev_num = prev.parse::<u64>().ok();

    let (path, start_line, end_line) = match prev_num {
        Some(prev_num) => {
            if prev_num == 0 {
                anyhow::bail!("line numbers must be >= 1");
            }
            parts.pop();
            let path = parts.join(":").trim().to_string();
            (path, Some(prev_num), Some(last_num))
        }
        None => {
            let path = parts.join(":").trim().to_string();
            (path, Some(last_num), None)
        }
    };

    if path.is_empty() {
        anyhow::bail!("@file path must be non-empty");
    }
    if let (Some(start), Some(end)) = (start_line, end_line) {
        if end < start {
            anyhow::bail!("end_line must be >= start_line");
        }
    }

    Ok((path, start_line, end_line))
}
