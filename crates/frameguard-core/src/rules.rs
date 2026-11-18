use anyhow::{bail, Context, Result};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Plain,
    Regex,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Rule {
    pub id: u64,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: RuleKind,
    pub pattern: String,
    pub risk: Risk,
    pub secret: bool,
    pub enabled: bool,
}

pub fn default_rules() -> Vec<Rule> {
    use Risk::*;
    use RuleKind::*;
    [
        ("key_file", Plain, "key_file", Low, false, true),
        ("api_keys", Plain, "api_keys", Medium, false, true),
        (
            "API key label + value",
            Regex,
            r#"(?:api[ _-]?key|secret|token)\s*[:=]\s*["']?[A-Z0-9_\-]{16,}"#,
            High,
            true,
            true,
        ),
        (
            "Email address",
            Regex,
            r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}",
            Medium,
            true,
            true,
        ),
        (
            "Password assignment",
            Regex,
            r"(?:password|passwd|pwd)\s*[:=]\s*\S+",
            High,
            true,
            true,
        ),
        (
            "Bearer token",
            Regex,
            r"Bearer\s+[A-Z0-9._\-]{20,}",
            High,
            true,
            true,
        ),
        (
            "AWS access key ID",
            Regex,
            r"\bAKIA[A-Z0-9]{16}\b",
            High,
            true,
            true,
        ),
        (
            "Private key header",
            Regex,
            r"-----BEGIN (?:[A-Z]+ )*PRIVATE KEY-----",
            High,
            false,
            true,
        ),
        (".env file", Plain, ".env", Low, false, false),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, (name, kind, pattern, risk, secret, enabled))| Rule {
            id: index as u64 + 1,
            name: name.into(),
            kind,
            pattern: pattern.into(),
            risk,
            secret,
            enabled,
        },
    )
    .collect()
}

pub fn test_rule(rule: &Rule, sample: &str) -> Result<Vec<[usize; 2]>> {
    Ok(compile(rule.kind, &rule.pattern)?
        .find_iter(sample)
        .filter(|matched| !matched.is_empty())
        .map(|matched| [matched.start(), matched.end()])
        .collect())
}

fn compile(kind: RuleKind, pattern: &str) -> Result<Regex> {
    if pattern.trim().is_empty() {
        bail!("Enter a pattern.");
    }
    let pattern = match kind {
        RuleKind::Plain => regex::escape(pattern),
        RuleKind::Regex => pattern.to_owned(),
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()
        .context("Invalid rule pattern")
}

pub(crate) struct Matcher {
    pub name: String,
    pub risk: Risk,
    pub secret: bool,
    pub custom: bool,
    pub regex: Regex,
}

pub(crate) fn matchers(rules: &[Rule], phrases: &[String]) -> Result<Vec<Matcher>> {
    let mut compiled = Vec::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        compiled.push(Matcher {
            name: rule.name.clone(),
            risk: rule.risk,
            secret: rule.secret,
            custom: false,
            regex: compile(rule.kind, &rule.pattern)
                .with_context(|| format!("Rule {}", rule.name))?,
        });
    }
    for phrase in phrases
        .iter()
        .map(|phrase| phrase.trim())
        .filter(|phrase| !phrase.is_empty())
    {
        compiled.push(Matcher {
            name: "Custom text".into(),
            risk: Risk::Medium,
            secret: false,
            custom: true,
            regex: compile(RuleKind::Plain, phrase)?,
        });
    }
    if compiled.is_empty() {
        bail!("Enable a rule or add custom text before scanning.");
    }
    Ok(compiled)
}
