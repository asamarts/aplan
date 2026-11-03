//! Human-sized explanations of enumerable names (`A1-198`).
//!
//! [`crate::diagnostic::Rule`] already carries the stable code, severity and
//! invariant that generate both `facts.json` and each heading in
//! `docs/rule-catalogue.md`. An explanation projects that same row. It does not
//! copy the corpus's example diagnostics: those contain normalised fixture
//! values, while the finding a reader arrived from already carries the real
//! message and repair instruction for their plan.

use crate::diagnostic::{Rule, Severity};

/// The stable meaning of one diagnostic rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleExplanation {
    /// The stable code printed by a finding.
    pub code: &'static str,
    /// Whether the finding fails by itself or is policy-controlled.
    pub severity: Severity,
    /// The invariant the rule enforces, shared with the rule catalogue.
    pub title: &'static str,
}

impl From<Rule> for RuleExplanation {
    fn from(rule: Rule) -> Self {
        Self {
            code: rule.code(),
            severity: rule.severity(),
            title: rule.title(),
        }
    }
}

/// Explain an exact stable rule code.
///
/// Matching is deliberately exact. A diagnostic prints the canonical spelling,
/// so copy and paste is the ordinary path; silently normalising another name
/// would make capability identity less precise.
#[must_use]
pub fn explain_rule(code: &str) -> Option<RuleExplanation> {
    Rule::from_code(code).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_projects_its_authoritative_row_and_nothing_else_does() {
        for &rule in Rule::ALL {
            assert_eq!(
                explain_rule(rule.code()),
                Some(RuleExplanation {
                    code: rule.code(),
                    severity: rule.severity(),
                    title: rule.title(),
                })
            );
        }
        assert_eq!(explain_rule("APLAN999"), None);
        assert_eq!(explain_rule("aplan012"), None);
    }
}
