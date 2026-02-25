use std::collections::HashMap;
use std::sync::Arc;

use crate::decision::Decision;
use crate::error::{Error, Result};
use crate::rule::{PatternToken, PrefixPattern, PrefixRule, RuleMatch, RuleRef};

type HeuristicsFallback<'a> = Option<&'a dyn Fn(&[String]) -> Decision>;

#[derive(Clone, Debug)]
pub struct Policy {
    rules_by_program: HashMap<String, Vec<RuleRef>>,
}

impl Policy {
    pub fn new(rules_by_program: HashMap<String, Vec<RuleRef>>) -> Self {
        Self { rules_by_program }
    }

    pub fn empty() -> Self {
        Self::new(HashMap::new())
    }

    pub fn rules(&self) -> &HashMap<String, Vec<RuleRef>> {
        &self.rules_by_program
    }

    pub fn add_rule(&mut self, rule: RuleRef) {
        self.rules_by_program
            .entry(rule.program().to_string())
            .or_default()
            .push(rule);
    }

    pub fn add_prefix_rule(&mut self, prefix: &[String], decision: Decision) -> Result<()> {
        let (first_token, rest) = prefix
            .split_first()
            .ok_or_else(|| Error::InvalidPattern("prefix cannot be empty".to_string()))?;

        let rule: RuleRef = Arc::new(PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from(first_token.as_str()),
                rest: rest
                    .iter()
                    .map(|token| PatternToken::Single(token.clone()))
                    .collect::<Vec<_>>()
                    .into(),
            },
            decision,
            justification: None,
        });

        self.add_rule(rule);
        Ok(())
    }

    pub fn check<F>(&self, cmd: &[String], heuristics_fallback: &F) -> Evaluation
    where
        F: Fn(&[String]) -> Decision,
    {
        let matched_rules = self.matches_for_command(cmd, Some(heuristics_fallback));
        Evaluation::from_matches(matched_rules)
    }

    pub fn check_multiple<Commands, F>(
        &self,
        commands: Commands,
        heuristics_fallback: &F,
    ) -> Evaluation
    where
        Commands: IntoIterator,
        Commands::Item: AsRef<[String]>,
        F: Fn(&[String]) -> Decision,
    {
        let matched_rules: Vec<RuleMatch> = commands
            .into_iter()
            .flat_map(|command| {
                self.matches_for_command(command.as_ref(), Some(heuristics_fallback))
            })
            .collect();

        Evaluation::from_matches(matched_rules)
    }

    pub fn matches_for_command(
        &self,
        cmd: &[String],
        heuristics_fallback: HeuristicsFallback<'_>,
    ) -> Vec<RuleMatch> {
        let matched_rules: Vec<RuleMatch> = match cmd.first() {
            Some(first) => self
                .rules_by_program
                .get(first)
                .map(|rules| rules.iter().filter_map(|rule| rule.matches(cmd)).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };

        if matched_rules.is_empty()
            && let Some(heuristics_fallback) = heuristics_fallback
        {
            vec![RuleMatch::HeuristicsRuleMatch {
                command: cmd.to_vec(),
                decision: heuristics_fallback(cmd),
            }]
        } else {
            matched_rules
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    pub decision: Decision,
    #[serde(rename = "matchedRules")]
    pub matched_rules: Vec<RuleMatch>,
}

impl Evaluation {
    fn from_matches(matched_rules: Vec<RuleMatch>) -> Self {
        let decision = matched_rules
            .iter()
            .map(RuleMatch::decision)
            .max()
            .unwrap_or(Decision::Prompt);

        Self {
            decision,
            matched_rules,
        }
    }
}
