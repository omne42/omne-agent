use super::*;

    #[test]
    fn append_fan_out_linkage_issue_markdown_behaves_for_present_and_empty_values() {
        let mut with_issue = String::new();
        append_fan_out_linkage_issue_markdown(
            &mut with_issue,
            Some("fan-out linkage issue: task_id=t1 status=Failed"),
        );
        assert!(with_issue.contains("## Fan-out Linkage Issue"));
        assert!(with_issue.contains("task_id=t1"));

        let mut without_issue = String::new();
        append_fan_out_linkage_issue_markdown(&mut without_issue, Some("   "));
        assert!(without_issue.is_empty());
    }

    #[test]
    fn fan_out_linkage_issue_artifact_write_params_use_expected_type() {
        let parent_thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_artifact_write_params(
            parent_thread_id,
            Some(parent_turn_id),
            fan_in_artifact_id,
            "fan-out linkage issue: task_id=t1 status=Failed",
        )
        .expect("params");
        let params = artifact_write_params_json(params);

        assert_eq!(
            params["artifact_type"].as_str(),
            Some("fan_out_linkage_issue")
        );
        assert_eq!(params["summary"].as_str(), Some("fan-out linkage issue"));
        let parent_thread_id_s = parent_thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(parent_thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        assert!(
            params["text"]
                .as_str()
                .is_some_and(|text| text.contains(&fan_in_artifact_id.to_string()))
        );
        assert!(
            params["text"].as_str().is_some_and(|text| text.contains("## Structured Data"))
        );
        assert!(params["text"]
            .as_str()
            .is_some_and(|text| text.contains("\"schema_version\": \"fan_out_linkage_issue.v1\"")));
        assert!(
            fan_out_linkage_issue_artifact_write_params(
                parent_thread_id,
                None,
                fan_in_artifact_id,
                "  ",
            )
            .is_none()
        );
    }

    #[test]
    fn fan_out_linkage_issue_artifact_write_params_allow_null_turn_id() {
        let parent_thread_id = ThreadId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_artifact_write_params(
            parent_thread_id,
            None,
            fan_in_artifact_id,
            "fan-out linkage issue",
        )
        .expect("params");
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_out_linkage_issue_clear_artifact_write_params_use_clear_type() {
        let parent_thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_clear_artifact_write_params(
            parent_thread_id,
            Some(parent_turn_id),
            fan_in_artifact_id,
        );
        let params = artifact_write_params_json(params);

        assert_eq!(
            params["artifact_type"].as_str(),
            Some("fan_out_linkage_issue_clear")
        );
        assert_eq!(
            params["summary"].as_str(),
            Some("fan-out linkage issue cleared")
        );
        let parent_thread_id_s = parent_thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(parent_thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        assert!(
            params["text"]
                .as_str()
                .is_some_and(|text| text.contains(&fan_in_artifact_id.to_string()))
        );
        assert!(params["text"]
            .as_str()
            .is_some_and(|text| text.contains("## Structured Data")));
        assert!(params["text"].as_str().is_some_and(|text| {
            text.contains("\"schema_version\": \"fan_out_linkage_issue_clear.v1\"")
        }));
    }

    #[test]
    fn fan_out_linkage_issue_clear_artifact_write_params_allow_null_turn_id() {
        let parent_thread_id = ThreadId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_clear_artifact_write_params(
            parent_thread_id,
            None,
            fan_in_artifact_id,
        );
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_out_result_error_artifact_write_params_include_parent_turn_id_when_present() {
        let parent_thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let params = fan_out_result_error_artifact_write_params(
            parent_thread_id,
            Some(parent_turn_id),
            "fan-out result artifact write failed: t1".to_string(),
            "error body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert_eq!(
            params["artifact_type"].as_str(),
            Some("fan_out_result_error")
        );
        let parent_thread_id_s = parent_thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(parent_thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        assert_eq!(
            params["summary"].as_str(),
            Some("fan-out result artifact write failed: t1")
        );
        assert_eq!(params["text"].as_str(), Some("error body"));
    }

    #[test]
    fn fan_out_result_error_artifact_write_params_allow_null_turn_id() {
        let params = fan_out_result_error_artifact_write_params(
            ThreadId::new(),
            None,
            "fan-out result artifact write failed: t1".to_string(),
            "error body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_in_summary_artifact_write_params_include_parent_turn_id_when_present() {
        let thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let artifact_id = ArtifactId::new();
        let params = fan_in_summary_artifact_write_params(
            thread_id,
            Some(parent_turn_id),
            artifact_id,
            "summary body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert_eq!(params["artifact_type"].as_str(), Some("fan_in_summary"));
        assert_eq!(params["summary"].as_str(), Some("fan-in summary"));
        let thread_id_s = thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        let artifact_id_s = artifact_id.to_string();
        assert_eq!(params["artifact_id"].as_str(), Some(artifact_id_s.as_str()));
        assert_eq!(params["text"].as_str(), Some("summary body"));
    }

    #[test]
    fn fan_in_summary_artifact_write_params_use_null_turn_id_when_absent() {
        let thread_id = ThreadId::new();
        let artifact_id = ArtifactId::new();
        let params = fan_in_summary_artifact_write_params(
            thread_id,
            None,
            artifact_id,
            "summary body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_in_related_artifact_write_params_keep_parent_turn_id_consistent_across_types() {
        let thread_id = ThreadId::new();
        let thread_id_s = thread_id.to_string();
        let artifact_id = ArtifactId::new();
        let artifact_id_s = artifact_id.to_string();
        let parent_turn_id = TurnId::new();
        let parent_turn_id_s = parent_turn_id.to_string();

        let summary_params = artifact_write_params_json(fan_in_summary_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
            "summary body".to_string(),
        ));
        let progress_params = artifact_write_params_json(fan_in_summary_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
            "progress body".to_string(),
        ));
        let linkage_issue_params = fan_out_linkage_issue_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
            "fan-out linkage issue",
        )
        .expect("linkage issue params");
        let linkage_issue_params = artifact_write_params_json(linkage_issue_params);
        let linkage_clear_params = artifact_write_params_json(
            fan_out_linkage_issue_clear_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
        ));
        let result_error_params = artifact_write_params_json(
            fan_out_result_error_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            "fan-out result artifact write failed: t1".to_string(),
            "error body".to_string(),
        ));

        for params in [
            &summary_params,
            &progress_params,
            &linkage_issue_params,
            &linkage_clear_params,
            &result_error_params,
        ] {
            assert_eq!(params["thread_id"].as_str(), Some(thread_id_s.as_str()));
            assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        }
        assert_eq!(summary_params["artifact_id"].as_str(), Some(artifact_id_s.as_str()));
        assert_eq!(progress_params["artifact_id"].as_str(), Some(artifact_id_s.as_str()));
        assert!(linkage_issue_params["artifact_id"].is_null());
        assert!(linkage_clear_params["artifact_id"].is_null());
    }
