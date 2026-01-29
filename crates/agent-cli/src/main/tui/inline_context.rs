    struct InlineContext {
        kind: InlinePaletteKind,
        query: String,
    }

    fn last_line_bounds(input: &str) -> (usize, &str) {
        match input.rfind('\n') {
            Some(idx) => (idx + 1, &input[idx + 1..]),
            None => (0, input),
        }
    }

    fn parse_inline_context(input: &str) -> Option<InlineContext> {
        let ends_with_whitespace = input.chars().last().is_some_and(char::is_whitespace);
        let (_line_start, line) = last_line_bounds(input);
        let line = line.trim_end();
        if line.is_empty() {
            return None;
        }

        if line.starts_with('/') {
            let body = line.trim_start_matches('/');
            let body = body.trim_start();
            if body.is_empty() {
                return Some(InlineContext {
                    kind: InlinePaletteKind::Command,
                    query: String::new(),
                });
            }
            let mut parts = body.splitn(2, char::is_whitespace);
            let token = parts.next().unwrap_or("").trim();
            let rest = parts.next().unwrap_or("").trim_start();
            if token.is_empty() {
                return Some(InlineContext {
                    kind: InlinePaletteKind::Command,
                    query: String::new(),
                });
            }
            let kind = match token {
                "mode" => InlinePaletteKind::Role,
                "model" => InlinePaletteKind::Model,
                "approval-policy" => InlinePaletteKind::ApprovalPolicy,
                "sandbox-policy" => InlinePaletteKind::SandboxPolicy,
                "sandbox-network" => InlinePaletteKind::SandboxNetworkAccess,
                _ => InlinePaletteKind::Command,
            };
            let query = match kind {
                InlinePaletteKind::Command => token.to_string(),
                _ => rest.to_string(),
            };
            return Some(InlineContext { kind, query });
        }

        if ends_with_whitespace {
            return None;
        }

        let token = line
            .split_whitespace()
            .last()
            .unwrap_or("")
            .trim_end_matches('\n');
        let mut token_chars = token.chars();
        let prefix = token_chars.next()?;
        let query: String = token_chars.collect();
        let kind = match prefix {
            '@' => InlinePaletteKind::Role,
            '$' => InlinePaletteKind::Skill,
            _ => return None,
        };
        Some(InlineContext { kind, query })
    }

    fn inline_token_span(input: &str, trigger: char) -> Option<(usize, usize)> {
        let (line_start, line) = last_line_bounds(input);
        let line_trimmed = line.trim_end();
        if line_trimmed.is_empty() {
            return None;
        }
        let token_start = line_trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let token = &line_trimmed[token_start..];
        if !token.starts_with(trigger) {
            return None;
        }
        Some((line_start + token_start, line_start + line_trimmed.len()))
    }

