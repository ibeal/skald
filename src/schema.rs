//! The store's frontmatter contract, declared in `.schema.toml` beside its tickets.
//!
//! The contract lives in the store rather than in skald because stores are independent: the personal
//! store's `spec`/`projects` and the work store's `linear`/`services` share no configuration and
//! neither knows the other exists. skald owns the *shape* of a contract, never its contents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::errors::{Result, SkaldError};

/// The schema filename, always at the store root. There is no upward search: discovering a contract
/// from somewhere other than the store it governs is the same class of guess as falling back to the
/// current directory for the store itself.
pub const SCHEMA_FILE: &str = ".schema.toml";

// The contract's *shape* is skald's to define, and it is defined here because the read side already
// depends on knowing which keys a store declares. `check` and the write commands are what read the
// rules; a store authored today is then already valid when they land.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(default)]
    pub store: StoreConfig,
    /// Declared frontmatter keys, by name. Key *order* is deliberately absent: `new` scaffolds from
    /// the store's template, so the template is the single source of truth for order.
    #[serde(default)]
    pub keys: BTreeMap<String, KeyRule>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreConfig {
    /// Template `new` scaffolds from, relative to the store root.
    pub template: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRule {
    #[serde(rename = "type")]
    pub kind: KeyKind,
    /// The key must be present. Presence and emptiness are separate: `pr` is always present and
    /// usually empty.
    #[serde(default)]
    pub required: bool,
    /// Permitted values for `type = "enum"` and `type = "enum-list"`.
    #[serde(default)]
    pub values: Vec<String>,
    /// Whether `key:` with nothing after it is acceptable. Defaults to true — an unfilled field is
    /// the normal state of a ticket mid-flight, not a violation.
    #[serde(default = "default_allow_empty")]
    pub allow_empty: bool,
    pub description: Option<String>,
}

fn default_allow_empty() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum KeyKind {
    String,
    /// One value from `values`, with nothing appended. This is the rule the whole tool exists for.
    Enum,
    /// `YYYY-MM-DD`.
    Date,
    /// An absolute `http(s)` URL, or empty.
    Url,
    List,
    /// A list whose every item comes from `values`.
    EnumList,
}

impl Schema {
    /// Read the schema at a store root. `Ok(None)` when there is none: a store without a declared
    /// contract is still readable, and only the operations that need validation fail.
    pub fn read(store_root: &Path) -> Result<Option<Self>> {
        let path = Self::path(store_root);
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(SkaldError::ReadSchema(path, source)),
        };
        toml::from_str(&source)
            .map(Some)
            .map_err(|error| SkaldError::ParseSchema(path, error))
    }

    pub fn path(store_root: &Path) -> PathBuf {
        store_root.join(SCHEMA_FILE)
    }

    pub fn declared_keys(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    pub fn declares(&self, key: &str) -> bool {
        self.keys.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyKind, Schema};

    const SOURCE: &str = r#"
[store]
template = "_TICKET_TEMPLATE.md"

[keys.id]
type = "string"
required = true

[keys.phase]
type = "enum"
required = true
allow_empty = false
values = ["intake", "build", "review", "merged", "blocked"]

[keys.pr]
type = "url"
required = true

[keys.projects]
type = "list"
"#;

    #[test]
    fn a_schema_declares_kinds_requirements_and_enum_values() {
        let schema: Schema = toml::from_str(SOURCE).unwrap();
        assert_eq!(
            schema.store.template.as_deref(),
            Some("_TICKET_TEMPLATE.md")
        );
        assert_eq!(
            schema.declared_keys(),
            vec!["id", "phase", "pr", "projects"]
        );

        let phase = &schema.keys["phase"];
        assert_eq!(phase.kind, KeyKind::Enum);
        assert!(phase.required);
        assert!(!phase.allow_empty);
        assert!(phase.values.contains(&"review".to_string()));
    }

    #[test]
    fn an_unfilled_field_is_allowed_unless_the_store_says_otherwise() {
        let schema: Schema = toml::from_str(SOURCE).unwrap();
        // `pr:` is empty for most of a ticket's life, so emptiness defaults to permitted.
        assert!(schema.keys["pr"].allow_empty);
        assert!(!schema.keys["phase"].allow_empty);
    }

    #[test]
    fn a_typo_in_the_contract_is_an_error_rather_than_a_silent_default() {
        // Without `deny_unknown_fields`, `requird = true` would parse as "not required" and the
        // store would quietly stop enforcing the key it meant to enforce.
        let error = toml::from_str::<Schema>("[keys.id]\ntype = \"string\"\nrequird = true\n");
        assert!(error.is_err());
    }
}
