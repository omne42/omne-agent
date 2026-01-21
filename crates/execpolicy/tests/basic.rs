use std::any::Any;
use std::sync::Arc;

use pm_execpolicy::{Decision, Error, Evaluation, Policy, PolicyParser, RuleMatch, RuleRef};
use pm_execpolicy::{PatternToken, PrefixPattern, PrefixRule};

fn tokens(cmd: &[&str]) -> Vec<String> {
    cmd.iter().map(std::string::ToString::to_string).collect()
}

fn allow_all(_: &[String]) -> Decision {
    Decision::Allow
}

fn prompt_all(_: &[String]) -> Decision {
    Decision::Prompt
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuleSnapshot {
    Prefix(PrefixRule),
}

fn rule_snapshots(rules: &[RuleRef]) -> Vec<RuleSnapshot> {
    rules
        .iter()
        .map(|rule| {
            let rule_any = rule.as_ref() as &dyn Any;
            if let Some(prefix_rule) = rule_any.downcast_ref::<PrefixRule>() {
                RuleSnapshot::Prefix(prefix_rule.clone())
            } else {
                panic!("unexpected rule type in RuleRef: {rule:?}");
            }
        })
        .collect()
}

#[test]
fn basic_match() -> anyhow::Result<()> {
    let policy_src = r#"
prefix_rule(
    pattern = ["git", "status"],
)
"#;
    let mut parser = PolicyParser::new();
    parser.parse("test.rules", policy_src)?;
    let policy = parser.build();

    let cmd = tokens(&["git", "status"]);
    let evaluation = policy.check(&cmd, &allow_all);
    assert_eq!(
        Evaluation {
            decision: Decision::Allow,
            matched_rules: vec![RuleMatch::PrefixRuleMatch {
                matched_prefix: tokens(&["git", "status"]),
                decision: Decision::Allow,
                justification: None,
            }],
        },
        evaluation
    );
    Ok(())
}

#[test]
fn add_prefix_rule_rejects_empty_prefix() -> anyhow::Result<()> {
    let mut policy = Policy::empty();
    let result = policy.add_prefix_rule(&[], Decision::Allow);

    match result.unwrap_err() {
        Error::InvalidPattern(message) => assert_eq!(message, "prefix cannot be empty"),
        other => panic!("expected InvalidPattern(..), got {other:?}"),
    }
    Ok(())
}

#[test]
fn only_first_token_alias_expands_to_multiple_rules() -> anyhow::Result<()> {
    let policy_src = r#"
prefix_rule(
    pattern = [["bash", "sh"], ["-c", "-l"]],
)
"#;
    let mut parser = PolicyParser::new();
    parser.parse("test.rules", policy_src)?;
    let policy = parser.build();

    let bash_rules = rule_snapshots(
        policy
            .rules()
            .get("bash")
            .expect("missing bash rules")
            .as_slice(),
    );
    let sh_rules = rule_snapshots(
        policy
            .rules()
            .get("sh")
            .expect("missing sh rules")
            .as_slice(),
    );

    assert_eq!(
        vec![RuleSnapshot::Prefix(PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from("bash"),
                rest: vec![PatternToken::Alts(vec!["-c".to_string(), "-l".to_string()])].into(),
            },
            decision: Decision::Allow,
            justification: None,
        })],
        bash_rules
    );
    assert_eq!(
        vec![RuleSnapshot::Prefix(PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from("sh"),
                rest: vec![PatternToken::Alts(vec!["-c".to_string(), "-l".to_string()])].into(),
            },
            decision: Decision::Allow,
            justification: None,
        })],
        sh_rules
    );

    let bash_eval = policy.check(&tokens(&["bash", "-c", "echo", "hi"]), &allow_all);
    assert_eq!(
        Evaluation {
            decision: Decision::Allow,
            matched_rules: vec![RuleMatch::PrefixRuleMatch {
                matched_prefix: tokens(&["bash", "-c"]),
                decision: Decision::Allow,
                justification: None,
            }],
        },
        bash_eval
    );
    Ok(())
}

#[test]
fn match_and_not_match_examples_are_enforced() -> anyhow::Result<()> {
    let policy_src = r#"
prefix_rule(
    pattern = ["git", "status"],
    match = [["git", "status"], "git status"],
    not_match = [
        ["git", "--config", "color.status=always", "status"],
        "git --config color.status=always status",
    ],
)
"#;
    let mut parser = PolicyParser::new();
    parser.parse("test.rules", policy_src)?;
    Ok(())
}

#[test]
fn heuristics_match_is_returned_when_no_policy_matches() {
    let policy = Policy::empty();
    let command = tokens(&["python"]);

    let evaluation = policy.check(&command, &prompt_all);
    assert_eq!(
        Evaluation {
            decision: Decision::Prompt,
            matched_rules: vec![RuleMatch::HeuristicsRuleMatch {
                command,
                decision: Decision::Prompt,
            }],
        },
        evaluation
    );
}
