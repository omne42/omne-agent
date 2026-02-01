#[cfg(test)]
mod special_directives_tests {
    use super::*;

    #[test]
    fn split_special_directives_noop_without_directives() -> anyhow::Result<()> {
        let input = "\n\nhello\nworld\n";
        let (remaining, refs, attachments) = split_special_directives(input)?;
        assert_eq!(remaining, input);
        assert!(refs.is_empty());
        assert!(attachments.is_empty());
        Ok(())
    }

    #[test]
    fn split_special_directives_parses_file_and_diff() -> anyhow::Result<()> {
        let input = "@file crates/core/src/redaction.rs:1:3\n@diff\n\nplease help\n";
        let (remaining, refs, attachments) = split_special_directives(input)?;
        assert_eq!(remaining, "please help");
        assert_eq!(refs.len(), 2);
        assert!(attachments.is_empty());
        assert!(matches!(
            &refs[0],
            omne_agent_protocol::ContextRef::File(omne_agent_protocol::ContextRefFile {
                path,
                start_line: Some(1),
                end_line: Some(3),
                ..
            }) if path == "crates/core/src/redaction.rs"
        ));
        assert!(matches!(&refs[1], omne_agent_protocol::ContextRef::Diff(_)));
        Ok(())
    }

    #[test]
    fn split_special_directives_rejects_diff_args() {
        let err = split_special_directives("@diff nope\nx").unwrap_err();
        assert!(err.to_string().contains("@diff"));
    }

    #[test]
    fn split_special_directives_rejects_file_without_path() {
        let err = split_special_directives("@file\nx").unwrap_err();
        assert!(err.to_string().contains("@file"));
    }

    #[test]
    fn split_special_directives_parses_image_and_pdf() -> anyhow::Result<()> {
        let input = "@image assets/example.png\n@pdf https://example.com/file.pdf\n\nhello";
        let (remaining, refs, attachments) = split_special_directives(input)?;
        assert_eq!(remaining, "hello");
        assert!(refs.is_empty());
        assert!(matches!(
            &attachments[0],
            omne_agent_protocol::TurnAttachment::Image(omne_agent_protocol::TurnAttachmentImage {
                source: omne_agent_protocol::AttachmentSource::Path { path },
                ..
            }) if path == "assets/example.png"
        ));
        assert!(matches!(
            &attachments[1],
            omne_agent_protocol::TurnAttachment::File(omne_agent_protocol::TurnAttachmentFile {
                source: omne_agent_protocol::AttachmentSource::Url { url },
                media_type,
                ..
            }) if url == "https://example.com/file.pdf" && media_type == "application/pdf"
        ));
        Ok(())
    }
}
