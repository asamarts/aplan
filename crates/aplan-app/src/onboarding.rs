//! Typed first-use and repair states for interactive delivery adapters.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingCause {
    MissingPlan,
    MissingConfiguration,
    MissingRoot,
    UnreadableConfiguration,
    InvalidConfiguration,
}

impl OnboardingCause {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MissingPlan => "No plan discovered",
            Self::MissingConfiguration => "Selected configuration is missing",
            Self::MissingRoot => "Plan root is missing",
            Self::UnreadableConfiguration => "Configuration is unreadable",
            Self::InvalidConfiguration => "Configuration is invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationSelection {
    Discover {
        start: PathBuf,
        searched: Vec<PathBuf>,
        found: Option<PathBuf>,
    },
    Explicit {
        path: PathBuf,
    },
    None,
}

impl ConfigurationSelection {
    #[must_use]
    pub const fn trust_label(&self) -> &'static str {
        match self {
            Self::Discover { .. } => "repository configuration discovery",
            Self::Explicit { .. } => "explicit trusted configuration",
            Self::None => "no repository configuration (--no-config)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootSelection {
    Explicit,
    Configuration { file: PathBuf },
    Default,
}

impl RootSelection {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit --root",
            Self::Configuration { .. } => "selected configuration root",
            Self::Default => "default root",
        }
    }
}

/// One exact inert command shown as guidance. The TUI never executes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidedCommand {
    pub purpose: String,
    pub display: String,
}

/// The root and configuration choices applied to one interactive launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchResolution {
    pub plan_root: PathBuf,
    pub root_selection: RootSelection,
    pub configuration: ConfigurationSelection,
}

/// A classified launch failure that remains a stable interactive state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    pub cause: OnboardingCause,
    pub plan_root: PathBuf,
    pub root_selection: RootSelection,
    pub configuration: ConfigurationSelection,
    pub detail: String,
    pub guidance: Vec<String>,
    pub commands: Vec<GuidedCommand>,
}

impl Onboarding {
    #[must_use]
    pub fn resolution(&self) -> LaunchResolution {
        LaunchResolution {
            plan_root: self.plan_root.clone(),
            root_selection: self.root_selection.clone(),
            configuration: self.configuration.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_vocabulary_and_resolution_are_total() {
        let causes = [
            (OnboardingCause::MissingPlan, "No plan discovered"),
            (
                OnboardingCause::MissingConfiguration,
                "Selected configuration is missing",
            ),
            (OnboardingCause::MissingRoot, "Plan root is missing"),
            (
                OnboardingCause::UnreadableConfiguration,
                "Configuration is unreadable",
            ),
            (
                OnboardingCause::InvalidConfiguration,
                "Configuration is invalid",
            ),
        ];
        for (cause, label) in causes {
            assert_eq!(cause.label(), label);
        }

        let configuration = ConfigurationSelection::Explicit {
            path: PathBuf::from("selected.yml"),
        };
        for (selection, label) in [
            (
                ConfigurationSelection::Discover {
                    start: PathBuf::from("."),
                    searched: Vec::new(),
                    found: None,
                },
                "repository configuration discovery",
            ),
            (configuration.clone(), "explicit trusted configuration"),
            (
                ConfigurationSelection::None,
                "no repository configuration (--no-config)",
            ),
        ] {
            assert_eq!(selection.trust_label(), label);
        }

        for (selection, label) in [
            (RootSelection::Explicit, "explicit --root"),
            (
                RootSelection::Configuration {
                    file: PathBuf::from("selected.yml"),
                },
                "selected configuration root",
            ),
            (RootSelection::Default, "default root"),
        ] {
            assert_eq!(selection.label(), label);
        }

        let onboarding = Onboarding {
            cause: OnboardingCause::MissingPlan,
            plan_root: PathBuf::from("aplan"),
            root_selection: RootSelection::Explicit,
            configuration: configuration.clone(),
            detail: "missing".to_string(),
            guidance: Vec::new(),
            commands: Vec::new(),
        };
        assert_eq!(
            onboarding.resolution(),
            LaunchResolution {
                plan_root: PathBuf::from("aplan"),
                root_selection: RootSelection::Explicit,
                configuration,
            }
        );
    }
}
