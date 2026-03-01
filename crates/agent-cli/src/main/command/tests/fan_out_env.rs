use super::*;

#[test]
fn parse_env_bool_accepts_common_values() {
    assert_eq!(parse_bool_token("yes"), Some(true));
    assert_eq!(parse_bool_token("off"), Some(false));
    assert_eq!(parse_bool_token("  TRUE "), Some(true));
    assert_eq!(parse_bool_token("maybe"), None);
}
#[test]
fn fan_out_priority_aging_rounds_env_fallback_and_clamp() {
    let default_value =
        with_env_var("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", None, fan_out_priority_aging_rounds);
    assert_eq!(default_value, 3);

    let fallback_value = with_env_var(
        "OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS",
        Some("not-a-number"),
        fan_out_priority_aging_rounds,
    );
    assert_eq!(fallback_value, 3);

    let clamped_low = with_env_var(
        "OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS",
        Some("0"),
        fan_out_priority_aging_rounds,
    );
    assert_eq!(clamped_low, 1);

    let clamped_high = with_env_var(
        "OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS",
        Some("99999"),
        fan_out_priority_aging_rounds,
    );
    assert_eq!(clamped_high, 10_000);
}
#[test]
fn fan_out_scheduling_params_respects_unlimited_and_fixed_env_limit() {
    let unlimited = with_env_vars(
        &[
            ("OMNE_MAX_CONCURRENT_SUBAGENTS", Some("0")),
            ("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", Some("7")),
        ],
        || fan_out_scheduling_params(5),
    );
    assert_eq!(unlimited.env_max_concurrent_subagents, 0);
    assert_eq!(unlimited.effective_concurrency_limit, 5);
    assert_eq!(unlimited.priority_aging_rounds, 7);

    let unlimited_zero_tasks = with_env_var("OMNE_MAX_CONCURRENT_SUBAGENTS", Some("0"), || {
        fan_out_scheduling_params(0)
    });
    assert_eq!(unlimited_zero_tasks.env_max_concurrent_subagents, 0);
    assert_eq!(unlimited_zero_tasks.effective_concurrency_limit, 1);

    let fixed = with_env_var("OMNE_MAX_CONCURRENT_SUBAGENTS", Some("2"), || {
        fan_out_scheduling_params(5)
    });
    assert_eq!(fixed.env_max_concurrent_subagents, 2);
    assert_eq!(fixed.effective_concurrency_limit, 2);
}
