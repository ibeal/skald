//! Global, optional Skald configuration.

use std::{env, ffi::OsString, path::PathBuf};

use serde::Deserialize;

use crate::errors::{Result, SkaldError};

pub const CONFIG_FILE: &str = "config.toml";
const CONFIG_DIR: &str = "skald";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub store: Option<PathBuf>,
    pub state_change_webhook: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let Some(path) = path() else {
            return Ok(Self::default());
        };
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(SkaldError::ConfigRead(path, error)),
        };
        Self::parse(&source).map_err(|error| SkaldError::ConfigParse(path, error))
    }

    /// The environment is a process-local override. An explicitly empty variable disables the
    /// configured receiver, which preserves the former empty-means-off behavior.
    pub fn state_change_webhook(&self) -> Option<String> {
        self.webhook_with(env::var_os("STATE_CHANGE_WEBHOOK"))
    }

    pub fn store_with(&self, store_override: Option<OsString>) -> Option<PathBuf> {
        store_override
            .map(PathBuf::from)
            .or_else(|| self.store.clone())
    }

    fn webhook_with(&self, webhook_override: Option<OsString>) -> Option<String> {
        match webhook_override {
            Some(value) if value.is_empty() => None,
            Some(value) => value.into_string().ok(),
            None => self
                .state_change_webhook
                .clone()
                .filter(|url| !url.is_empty()),
        }
    }

    fn parse(source: &str) -> std::result::Result<Self, toml::de::Error> {
        toml::from_str(source)
    }
}

pub fn path() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
        .map(|root| root.join(CONFIG_DIR).join(CONFIG_FILE))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::Config;

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("unknown = true").is_err());
    }

    #[test]
    fn config_values_are_defaults_and_environment_values_override_them() {
        let config = Config::parse(
            "store = \"/configured/tickets\"\nstate_change_webhook = \"https://configured.test/events\"",
        )
        .unwrap();
        assert_eq!(
            config.store_with(None).unwrap(),
            PathBuf::from("/configured/tickets")
        );
        assert_eq!(
            config
                .store_with(Some(OsString::from("/override/tickets")))
                .unwrap(),
            PathBuf::from("/override/tickets")
        );
        assert_eq!(
            config.webhook_with(None).as_deref(),
            Some("https://configured.test/events")
        );
        assert_eq!(
            config
                .webhook_with(Some(OsString::from("https://override.test/events")))
                .as_deref(),
            Some("https://override.test/events")
        );
        assert_eq!(config.webhook_with(Some(OsString::new())), None);
    }
}
