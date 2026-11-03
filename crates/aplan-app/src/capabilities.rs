//! Typed build capability data shared by `aplan facts` and interactive views.
//!
//! The delivery adapter owns discovery of its clap and MCP surfaces, but once
//! discovered they travel as data. An interactive client therefore renders the
//! same inventory that the machine-readable command serialises instead of
//! parsing JSON or maintaining another command/rule list.

use serde::Serialize;

use crate::BuildIdentity;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BehaviourFact {
    pub name: String,
    pub what: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildCapabilityFact {
    pub enabled: bool,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandFact {
    pub about: Option<String>,
    pub default_format: Option<String>,
    pub formats: Vec<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpToolFact {
    pub description: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleFact {
    pub code: String,
    pub severity: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FactCounts {
    pub behaviours: usize,
    pub build_capabilities: usize,
    pub commands: usize,
    pub config_keys: usize,
    pub evidence_kinds: usize,
    pub formats: usize,
    pub mcp_tools: usize,
    pub rules: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExitCodeFacts {
    pub clean: u8,
    pub error: u8,
    pub internal: u8,
    pub usage: u8,
}

/// The exact typed payload emitted by `aplan facts`.
///
/// Field order intentionally matches the long-standing pretty-printed JSON so
/// moving from an untyped JSON builder does not churn the capability digest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BuildFacts {
    pub behaviours: Vec<BehaviourFact>,
    pub build_capabilities: Vec<BuildCapabilityFact>,
    pub commands: Vec<CommandFact>,
    pub config_keys: Vec<String>,
    pub counts: FactCounts,
    pub evidence_kinds: Vec<String>,
    pub exit_codes: ExitCodeFacts,
    pub formats: Vec<String>,
    pub mcp_tools: Vec<McpToolFact>,
    pub rules: Vec<RuleFact>,
    pub schema_version: String,
    pub version: String,
}

impl BuildFacts {
    /// The collection fields an About projection must deliberately dispose.
    pub const COLLECTION_KEYS: [&'static str; 8] = [
        "build_capabilities",
        "commands",
        "formats",
        "rules",
        "evidence_kinds",
        "behaviours",
        "mcp_tools",
        "config_keys",
    ];

    #[must_use]
    pub fn measured_counts(&self) -> FactCounts {
        FactCounts {
            rules: self.rules.len(),
            behaviours: self.behaviours.len(),
            formats: self.formats.len(),
            commands: self.commands.len(),
            evidence_kinds: self.evidence_kinds.len(),
            config_keys: self.config_keys.len(),
            mcp_tools: self.mcp_tools.len(),
            build_capabilities: self.build_capabilities.len(),
        }
    }

    #[must_use]
    pub fn capability_view(&self, identity: &BuildIdentity) -> CapabilityView {
        let groups = CapabilitySection::ALL
            .into_iter()
            .map(|section| CapabilityGroup {
                section,
                items: section.items(self),
            })
            .collect();
        CapabilityView {
            advertised_version: self.version.clone(),
            schema_version: self.schema_version.clone(),
            identity: identity.clone(),
            counts: self.counts,
            exit_codes: self.exit_codes,
            groups,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilitySection {
    BuildCapabilities,
    Commands,
    Formats,
    Rules,
    EvidenceKinds,
    Behaviours,
    McpTools,
    ConfigKeys,
}

impl CapabilitySection {
    pub const ALL: [Self; 8] = [
        Self::BuildCapabilities,
        Self::Commands,
        Self::Formats,
        Self::Rules,
        Self::EvidenceKinds,
        Self::Behaviours,
        Self::McpTools,
        Self::ConfigKeys,
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::BuildCapabilities => "build_capabilities",
            Self::Commands => "commands",
            Self::Formats => "formats",
            Self::Rules => "rules",
            Self::EvidenceKinds => "evidence_kinds",
            Self::Behaviours => "behaviours",
            Self::McpTools => "mcp_tools",
            Self::ConfigKeys => "config_keys",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BuildCapabilities => "Build capabilities",
            Self::Commands => "Commands",
            Self::Formats => "Formats",
            Self::Rules => "Rules",
            Self::EvidenceKinds => "Evidence kinds",
            Self::Behaviours => "Behaviours",
            Self::McpTools => "MCP tools",
            Self::ConfigKeys => "Configuration keys",
        }
    }

    fn items(self, facts: &BuildFacts) -> Vec<CapabilityItem> {
        match self {
            Self::BuildCapabilities => facts
                .build_capabilities
                .iter()
                .map(|fact| CapabilityItem {
                    name: fact.name.clone(),
                    detail: Some(if fact.enabled { "enabled" } else { "disabled" }.to_string()),
                })
                .collect(),
            Self::Commands => facts
                .commands
                .iter()
                .map(|fact| {
                    let about = fact.about.as_deref().unwrap_or("no description");
                    let formats = if fact.formats.is_empty() {
                        "no serialized formats".to_string()
                    } else {
                        format!("formats: {}", fact.formats.join(", "))
                    };
                    let default = fact
                        .default_format
                        .as_deref()
                        .map_or(String::new(), |value| format!("; default: {value}"));
                    CapabilityItem {
                        name: fact.name.clone(),
                        detail: Some(format!("{about}; {formats}{default}")),
                    }
                })
                .collect(),
            Self::Formats => plain_items(&facts.formats),
            Self::Rules => facts
                .rules
                .iter()
                .map(|fact| CapabilityItem {
                    name: fact.code.clone(),
                    detail: Some(format!("{}: {}", fact.severity, fact.title)),
                })
                .collect(),
            Self::EvidenceKinds => plain_items(&facts.evidence_kinds),
            Self::Behaviours => facts
                .behaviours
                .iter()
                .map(|fact| CapabilityItem {
                    name: fact.name.clone(),
                    detail: Some(fact.what.clone()),
                })
                .collect(),
            Self::McpTools => facts
                .mcp_tools
                .iter()
                .map(|fact| CapabilityItem {
                    name: fact.name.clone(),
                    detail: Some(fact.description.clone()),
                })
                .collect(),
            Self::ConfigKeys => plain_items(&facts.config_keys),
        }
    }
}

fn plain_items(values: &[String]) -> Vec<CapabilityItem> {
    values
        .iter()
        .map(|value| CapabilityItem {
            name: value.clone(),
            detail: None,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityItem {
    pub name: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGroup {
    pub section: CapabilitySection,
    pub items: Vec<CapabilityItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityView {
    pub advertised_version: String,
    pub schema_version: String,
    pub identity: BuildIdentity,
    pub counts: FactCounts,
    pub exit_codes: ExitCodeFacts,
    pub groups: Vec<CapabilityGroup>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // Every serialized leaf has a deliberate About disposition. This is kept
    // separate from the serialized shape on purpose: adding a field to a fact
    // type must fail this audit until its interactive treatment is chosen.
    const FIELD_DISPOSITIONS: &[(&str, &str)] = &[
        ("behaviours[].name", "group item name"),
        ("behaviours[].what", "group item detail"),
        ("build_capabilities[].enabled", "group item detail"),
        ("build_capabilities[].name", "group item name"),
        ("commands[].about", "group item detail"),
        ("commands[].default_format", "group item detail"),
        ("commands[].formats[]", "group item detail"),
        ("commands[].name", "group item name"),
        ("config_keys[]", "group item name"),
        ("counts.behaviours", "counts summary"),
        ("counts.build_capabilities", "counts summary"),
        ("counts.commands", "counts summary"),
        ("counts.config_keys", "counts summary"),
        ("counts.evidence_kinds", "counts summary"),
        ("counts.formats", "counts summary"),
        ("counts.mcp_tools", "counts summary"),
        ("counts.rules", "counts summary"),
        ("evidence_kinds[]", "group item name"),
        ("exit_codes.clean", "exit-code summary"),
        ("exit_codes.error", "exit-code summary"),
        ("exit_codes.internal", "exit-code summary"),
        ("exit_codes.usage", "exit-code summary"),
        ("formats[]", "group item name"),
        ("mcp_tools[].description", "group item detail"),
        ("mcp_tools[].name", "group item name"),
        ("rules[].code", "group item name"),
        ("rules[].severity", "group item detail"),
        ("rules[].title", "group item detail"),
        ("schema_version", "About header"),
        ("version", "About header"),
    ];

    fn serialized_leaf_paths(
        value: &serde_json::Value,
        prefix: &str,
        paths: &mut BTreeSet<String>,
    ) {
        match value {
            serde_json::Value::Object(object) => {
                if object.is_empty() && !prefix.is_empty() {
                    paths.insert(prefix.to_string());
                }
                for (key, value) in object {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    serialized_leaf_paths(value, &path, paths);
                }
            }
            serde_json::Value::Array(values) => {
                let path = format!("{prefix}[]");
                if values.is_empty() {
                    paths.insert(path.clone());
                }
                for value in values {
                    serialized_leaf_paths(value, &path, paths);
                }
            }
            _ => {
                paths.insert(prefix.to_string());
            }
        }
    }

    fn assert_projected_items(view: &CapabilityView) {
        let group = |section| {
            view.groups
                .iter()
                .find(|group| group.section == section)
                .expect("every section is projected")
        };
        assert_eq!(
            group(CapabilitySection::BuildCapabilities).items[0],
            CapabilityItem {
                name: "build.tui".to_string(),
                detail: Some("enabled".to_string()),
            }
        );
        assert_eq!(
            group(CapabilitySection::Commands).items[0],
            CapabilityItem {
                name: "check".to_string(),
                detail: Some("check a plan; formats: human; default: human".to_string()),
            }
        );
        assert_eq!(
            group(CapabilitySection::Rules).items[0],
            CapabilityItem {
                name: "APLAN001".to_string(),
                detail: Some("error: one rule".to_string()),
            }
        );
        assert_eq!(
            group(CapabilitySection::Behaviours).items[0],
            CapabilityItem {
                name: "behaviour.one".to_string(),
                detail: Some("does one thing".to_string()),
            }
        );
        assert_eq!(
            group(CapabilitySection::McpTools).items[0],
            CapabilityItem {
                name: "aplan_check".to_string(),
                detail: Some("check a plan".to_string()),
            }
        );
        for (section, expected) in [
            (CapabilitySection::Formats, "human"),
            (CapabilitySection::EvidenceKinds, "test"),
            (CapabilitySection::ConfigKeys, "root"),
        ] {
            assert_eq!(
                group(section).items[0],
                CapabilityItem {
                    name: expected.to_string(),
                    detail: None,
                }
            );
        }
    }

    fn facts() -> BuildFacts {
        BuildFacts {
            behaviours: vec![BehaviourFact {
                name: "behaviour.one".to_string(),
                what: "does one thing".to_string(),
            }],
            build_capabilities: vec![BuildCapabilityFact {
                name: "build.tui".to_string(),
                enabled: true,
            }],
            commands: vec![CommandFact {
                name: "check".to_string(),
                about: Some("check a plan".to_string()),
                formats: vec!["human".to_string()],
                default_format: Some("human".to_string()),
            }],
            config_keys: vec!["root".to_string()],
            counts: FactCounts {
                rules: 1,
                behaviours: 1,
                formats: 1,
                commands: 1,
                evidence_kinds: 1,
                config_keys: 1,
                mcp_tools: 1,
                build_capabilities: 1,
            },
            evidence_kinds: vec!["test".to_string()],
            exit_codes: ExitCodeFacts {
                clean: 0,
                error: 1,
                usage: 2,
                internal: 3,
            },
            formats: vec!["human".to_string()],
            mcp_tools: vec![McpToolFact {
                name: "aplan_check".to_string(),
                description: "check a plan".to_string(),
            }],
            rules: vec![RuleFact {
                code: "APLAN001".to_string(),
                severity: "error".to_string(),
                title: "one rule".to_string(),
            }],
            schema_version: "v1".to_string(),
            version: "0.test".to_string(),
        }
    }

    /// Proof for `tui.onboarding_and_capabilities_are_typed`: the interactive
    /// projection has one deliberate disposition for every typed facts leaf,
    /// including fields nested inside collections and scalar records.
    #[test]
    fn capability_projection_disposes_every_typed_fact_collection() {
        let facts = facts();
        assert_eq!(facts.counts, facts.measured_counts());
        let view = facts.capability_view(&BuildIdentity::default());
        assert_eq!(view.advertised_version, "0.test");
        assert_eq!(view.schema_version, "v1");
        assert_eq!(view.counts, facts.counts);
        assert_eq!(view.exit_codes, facts.exit_codes);
        assert_eq!(
            view.groups
                .iter()
                .map(|group| group.section.key())
                .collect::<Vec<_>>(),
            BuildFacts::COLLECTION_KEYS
        );
        assert!(view.groups.iter().all(|group| group.items.len() == 1));
        let serialized = serde_json::to_value(&facts).expect("facts serialize");
        let object = serialized.as_object().expect("facts are an object");
        let collection_keys = object
            .iter()
            .filter(|(_, value)| value.is_array())
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>();
        let mut projected_keys = BuildFacts::COLLECTION_KEYS.to_vec();
        projected_keys.sort_unstable();
        assert_eq!(collection_keys, projected_keys);
        assert_eq!(
            object
                .iter()
                .filter(|(_, value)| !value.is_array())
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            ["counts", "exit_codes", "schema_version", "version"]
        );

        let mut serialized_paths = BTreeSet::new();
        serialized_leaf_paths(&serialized, "", &mut serialized_paths);
        let disposition_paths = FIELD_DISPOSITIONS
            .iter()
            .map(|(path, disposition)| {
                assert!(!disposition.is_empty(), "{path} needs a view disposition");
                *path
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            disposition_paths.len(),
            FIELD_DISPOSITIONS.len(),
            "a facts leaf must have exactly one About disposition"
        );
        assert_eq!(
            serialized_paths,
            disposition_paths
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        );

        assert_projected_items(&view);
    }
}
