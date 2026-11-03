//! Atomic dependency additions and removals for existing tasks.

use serde::Deserialize;

/// All dependency operations for one target task.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The task whose `blocked_by` list changes.
    pub id: String,
    /// Prerequisites to add.
    #[serde(default)]
    pub add: Vec<String>,
    /// Prerequisites to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// A dependency manifest could not be read as a non-empty YAML list.
#[derive(Debug)]
pub enum Error {
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        said: String,
    },
    Empty {
        path: std::path::PathBuf,
    },
    Invalid {
        path: std::path::PathBuf,
        index: usize,
        said: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Self::Parse { path, said } => write!(
                f,
                "{} is not a dependency batch: {said}. Expected a YAML list of `id`, `add` and `remove` mappings",
                path.display()
            ),
            Self::Empty { path } => write!(f, "{} names no dependency operations", path.display()),
            Self::Invalid { path, index, said } => write!(
                f,
                "{} dependency entry {} is invalid: {said}",
                path.display(),
                index + 1
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. } | Self::Empty { .. } | Self::Invalid { .. } => None,
        }
    }
}

pub fn read(path: &std::path::Path) -> Result<(String, Vec<Entry>), Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let entries = parse(path, &text)?;
    Ok((text, entries))
}

pub fn parse(path: &std::path::Path, text: &str) -> Result<Vec<Entry>, Error> {
    let entries: Vec<Entry> = yaml_compat::from_str(text).map_err(|error| Error::Parse {
        path: path.to_path_buf(),
        said: error.to_string(),
    })?;
    if entries.is_empty() {
        return Err(Error::Empty {
            path: path.to_path_buf(),
        });
    }
    for (index, entry) in entries.iter().enumerate() {
        let invalid = if aplan_core::task::is_vacuous_ref(&entry.id) {
            Some("`id` may not be blank or visually empty")
        } else if entry.add.is_empty() && entry.remove.is_empty() {
            Some("give at least one `add` or `remove` prerequisite")
        } else if entry
            .add
            .iter()
            .chain(&entry.remove)
            .any(|id| aplan_core::task::is_vacuous_ref(id))
        {
            Some("prerequisite ids may not be blank or visually empty")
        } else {
            None
        };
        if let Some(said) = invalid {
            return Err(Error::Invalid {
                path: path.to_path_buf(),
                index,
                said: said.to_string(),
            });
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_mixed_dependency_operations_without_reordering_them() {
        let entries = parse(
            std::path::Path::new("edges.yml"),
            "- id: A-1\n  add: [A-2, A-3]\n  remove: [A-4]\n",
        )
        .expect("valid dependency manifest");
        assert_eq!(entries[0].id, "A-1");
        assert_eq!(entries[0].add, ["A-2", "A-3"]);
        assert_eq!(entries[0].remove, ["A-4"]);
    }

    #[test]
    fn refuses_unknown_fields_and_an_empty_list() {
        let path = std::path::Path::new("edges.yml");
        assert!(parse(path, "- id: A-1\n  adds: [A-2]\n").is_err());
        assert!(parse(path, "[]\n").is_err());
        assert!(parse(path, "- id: A-1\n").is_err());
    }
}
