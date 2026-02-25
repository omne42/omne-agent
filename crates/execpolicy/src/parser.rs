use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::sync::Arc;

use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::Value;
use starlark::values::list::{ListRef, UnpackList};
use starlark::values::none::NoneType;

use crate::decision::Decision;
use crate::error::{Error, Result};
use crate::policy::Policy;
use crate::rule::{
    PatternToken, PrefixPattern, PrefixRule, RuleRef, validate_match_examples,
    validate_not_match_examples,
};

pub struct PolicyParser {
    builder: RefCell<PolicyBuilder>,
}

impl Default for PolicyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyParser {
    pub fn new() -> Self {
        Self {
            builder: RefCell::new(PolicyBuilder::new()),
        }
    }

    pub fn parse(&mut self, policy_identifier: &str, policy_file_contents: &str) -> Result<()> {
        let mut dialect = Dialect::Extended.clone();
        dialect.enable_f_strings = true;
        let ast = AstModule::parse(
            policy_identifier,
            policy_file_contents.to_string(),
            &dialect,
        )
        .map_err(Error::Starlark)?;
        let globals = GlobalsBuilder::standard().with(policy_builtins).build();
        let module = Module::new();
        {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&self.builder);
            eval.eval_module(ast, &globals).map_err(Error::Starlark)?;
        }
        Ok(())
    }

    pub fn build(self) -> Policy {
        self.builder.into_inner().build()
    }
}

#[derive(Debug, ProvidesStaticType)]
struct PolicyBuilder {
    rules_by_program: HashMap<String, Vec<RuleRef>>,
}

impl PolicyBuilder {
    fn new() -> Self {
        Self {
            rules_by_program: HashMap::new(),
        }
    }

    fn add_rule(&mut self, rule: RuleRef) {
        self.rules_by_program
            .entry(rule.program().to_string())
            .or_default()
            .push(rule);
    }

    fn build(self) -> Policy {
        Policy::new(self.rules_by_program)
    }
}

fn parse_pattern<'v>(pattern: UnpackList<Value<'v>>) -> Result<Vec<PatternToken>> {
    let tokens: Vec<PatternToken> = pattern
        .items
        .into_iter()
        .map(parse_pattern_token)
        .collect::<Result<_>>()?;
    if tokens.is_empty() {
        Err(Error::InvalidPattern("pattern cannot be empty".to_string()))
    } else {
        Ok(tokens)
    }
}

fn parse_pattern_token<'v>(value: Value<'v>) -> Result<PatternToken> {
    if let Some(s) = value.unpack_str() {
        Ok(PatternToken::Single(s.to_string()))
    } else if let Some(list) = ListRef::from_value(value) {
        let tokens: Vec<String> = list
            .content()
            .iter()
            .map(|value| {
                value
                    .unpack_str()
                    .ok_or_else(|| {
                        Error::InvalidPattern(format!(
                            "pattern alternative must be a string (got {})",
                            value.get_type()
                        ))
                    })
                    .map(str::to_string)
            })
            .collect::<Result<_>>()?;

        match tokens.as_slice() {
            [] => Err(Error::InvalidPattern(
                "pattern alternatives cannot be empty".to_string(),
            )),
            [single] => Ok(PatternToken::Single(single.clone())),
            _ => Ok(PatternToken::Alts(tokens)),
        }
    } else {
        Err(Error::InvalidPattern(format!(
            "pattern element must be a string or list of strings (got {})",
            value.get_type()
        )))
    }
}

fn parse_examples<'v>(examples: UnpackList<Value<'v>>) -> Result<Vec<Vec<String>>> {
    examples.items.into_iter().map(parse_example).collect()
}

fn parse_example<'v>(value: Value<'v>) -> Result<Vec<String>> {
    if let Some(raw) = value.unpack_str() {
        parse_string_example(raw)
    } else if let Some(list) = ListRef::from_value(value) {
        parse_list_example(list)
    } else {
        Err(Error::InvalidExample(format!(
            "example must be a string or list of strings (got {})",
            value.get_type()
        )))
    }
}

fn parse_string_example(raw: &str) -> Result<Vec<String>> {
    let tokens = shlex::split(raw).ok_or_else(|| {
        Error::InvalidExample("example string has invalid shell syntax".to_string())
    })?;

    if tokens.is_empty() {
        Err(Error::InvalidExample(
            "example cannot be an empty string".to_string(),
        ))
    } else {
        Ok(tokens)
    }
}

fn parse_list_example(list: &ListRef) -> Result<Vec<String>> {
    let tokens: Vec<String> = list
        .content()
        .iter()
        .map(|value| {
            value
                .unpack_str()
                .ok_or_else(|| {
                    Error::InvalidExample(format!(
                        "example tokens must be strings (got {})",
                        value.get_type()
                    ))
                })
                .map(str::to_string)
        })
        .collect::<Result<_>>()?;

    if tokens.is_empty() {
        Err(Error::InvalidExample(
            "example cannot be an empty list".to_string(),
        ))
    } else {
        Ok(tokens)
    }
}

fn policy_builder<'v, 'a>(eval: &Evaluator<'v, 'a, '_>) -> Result<RefMut<'a, PolicyBuilder>> {
    let extra = eval.extra.as_ref().ok_or_else(|| {
        Error::InvalidRule("policy_builder requires Evaluator.extra to be populated".to_string())
    })?;
    let cell = extra
        .downcast_ref::<RefCell<PolicyBuilder>>()
        .ok_or_else(|| {
            Error::InvalidRule("Evaluator.extra must contain a PolicyBuilder".to_string())
        })?;
    Ok(cell.borrow_mut())
}

#[starlark_module]
fn policy_builtins(builder: &mut GlobalsBuilder) {
    fn prefix_rule<'v>(
        pattern: UnpackList<Value<'v>>,
        decision: Option<&'v str>,
        r#match: Option<UnpackList<Value<'v>>>,
        not_match: Option<UnpackList<Value<'v>>>,
        justification: Option<&'v str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let decision = match decision {
            Some(raw) => Decision::parse(raw)?,
            None => Decision::Allow,
        };

        let justification = match justification {
            Some(raw) if raw.trim().is_empty() => {
                return Err(Error::InvalidRule("justification cannot be empty".to_string()).into());
            }
            Some(raw) => Some(raw.to_string()),
            None => None,
        };

        let pattern_tokens = parse_pattern(pattern)?;

        let matches: Vec<Vec<String>> =
            r#match.map(parse_examples).transpose()?.unwrap_or_default();
        let not_matches: Vec<Vec<String>> = not_match
            .map(parse_examples)
            .transpose()?
            .unwrap_or_default();

        let mut builder = policy_builder(eval)?;

        let (first_token, remaining_tokens) = pattern_tokens
            .split_first()
            .ok_or_else(|| Error::InvalidPattern("pattern cannot be empty".to_string()))?;

        let rest: Arc<[PatternToken]> = remaining_tokens.to_vec().into();

        let rules: Vec<RuleRef> = first_token
            .alternatives()
            .iter()
            .map(|head| {
                Arc::new(PrefixRule {
                    pattern: PrefixPattern {
                        first: Arc::from(head.as_str()),
                        rest: rest.clone(),
                    },
                    decision,
                    justification: justification.clone(),
                }) as RuleRef
            })
            .collect();

        validate_not_match_examples(&rules, &not_matches)?;
        validate_match_examples(&rules, &matches)?;

        rules.into_iter().for_each(|rule| builder.add_rule(rule));
        Ok(NoneType)
    }
}
