use super::*;

#[test]
fn build_inbox_json_output_omits_summary_cache_stats_when_absent() -> anyhow::Result<()> {
    let output = build_inbox_json_output(1, 2, vec![], None)?;
    assert_eq!(output["prev_count"].as_u64(), Some(1));
    assert_eq!(output["cur_count"].as_u64(), Some(2));
    assert!(
        output["threads"]
            .as_array()
            .is_some_and(|rows| rows.is_empty())
    );
    assert!(output.get("summary_cache_stats").is_none());
    Ok(())
}

#[test]
fn build_inbox_json_output_includes_summary_cache_stats_when_present() -> anyhow::Result<()> {
    let stats = InboxSummaryCacheStats {
        fan_out_meta: 1,
        fan_out_cache_some: 2,
        fan_out_cache_none: 3,
        fan_out_attention: 4,
        fan_out_fetch_some: 5,
        fan_out_fetch_none: 6,
        fan_in_meta: 7,
        fan_in_cache_some: 8,
        fan_in_cache_none: 9,
        fan_in_attention: 10,
        fan_in_fetch_some: 11,
        fan_in_fetch_none: 12,
        fan_in_skip_unblocked: 13,
        fan_in_diag_meta: 14,
        fan_in_diag_cache_some: 15,
        fan_in_diag_cache_none: 16,
        fan_in_diag_attention: 17,
        fan_in_diag_fetch_some: 18,
        fan_in_diag_fetch_none: 19,
        fan_in_diag_skip_absent: 20,
        subagent_meta: 21,
        subagent_cache_some: 22,
        subagent_cache_none: 23,
        subagent_attention_some: 24,
        subagent_attention_none: 25,
        subagent_fetch_some: 26,
        subagent_fetch_none: 27,
        subagent_skip_no_pending: 28,
    };
    let output = build_inbox_json_output(3, 4, vec![], Some(&stats))?;
    assert_eq!(
        output["summary_cache_stats"]["fan_out_meta"].as_u64(),
        Some(1)
    );
    assert_eq!(
        output["summary_cache_stats"]["fan_in_meta"].as_u64(),
        Some(7)
    );
    assert_eq!(
        output["summary_cache_stats"]["fan_in_diag_meta"].as_u64(),
        Some(14)
    );
    assert_eq!(
        output["summary_cache_stats"]["subagent_meta"].as_u64(),
        Some(21)
    );
    assert_eq!(
        output["summary_cache_stats"]["subagent_skip_no_pending"].as_u64(),
        Some(28)
    );
    Ok(())
}
