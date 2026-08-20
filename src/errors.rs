use std::fmt::{self, Display};
use std::io;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, SkaldError>;

/// The environment variable naming the active ticket store. There is deliberately no default and no
/// search: see [`SkaldError::StoreUnset`].
pub const STORE_VAR: &str = "SKALD_STORE";

#[derive(Debug)]
pub enum SkaldError {
    Io(io::Error),
    SerializeJson(serde_json::Error),
    StoreUnset,
    StoreRelative(PathBuf),
    StoreMissing(PathBuf),
    StoreNotADirectory(PathBuf),
    TicketNotAFile(PathBuf),
    StoreUnreadable(PathBuf, io::Error),
    ReadSchema(PathBuf, io::Error),
    ParseSchema(PathBuf, toml::de::Error),
    SchemaMissing(PathBuf),
    ReadTicket(PathBuf, io::Error),
    TicketIdNotAName(String),
    UnknownTicket {
        id: String,
        candidates: Vec<String>,
    },
    UnknownSection {
        id: String,
        section: String,
        available: Vec<String>,
    },
    UndeclaredFilterKey {
        key: String,
        declared: Vec<String>,
    },
}

impl Display for SkaldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(source) => write!(f, "{source}"),
            Self::SerializeJson(source) => write!(f, "failed to serialize JSON: {source}"),
            // Guessing a store means writing a ticket into the wrong saga, which is worse than not
            // running at all. So the message teaches the fix instead of picking one.
            Self::StoreUnset => write!(
                f,
                "${STORE_VAR} is not set, so there is no active ticket store.\n\
                 Set it to the absolute path of a ticket directory, e.g.\n\
                 \x20 export {STORE_VAR}=/path/to/tickets\n\
                 skald never guesses a default and never falls back to the current directory."
            ),
            Self::StoreRelative(path) => write!(
                f,
                "${STORE_VAR} must be an absolute path, but is `{}`.\n\
                 A relative store resolves against whatever directory skald happens to run in, \
                 which is the same guess as having no store at all.",
                path.display()
            ),
            Self::StoreMissing(path) => write!(
                f,
                "${STORE_VAR} is `{}`, which does not exist.",
                path.display()
            ),
            Self::StoreNotADirectory(path) => write!(
                f,
                "${STORE_VAR} is `{}`, which is not a directory.",
                path.display()
            ),
            // skald is meant to be the only way into a store, so a link is a way out of one. That
            // matters most for the write commands, but a `show` that prints a file from outside the
            // store is already a hole in the containment the deny is supposed to provide.
            Self::TicketNotAFile(path) => write!(
                f,
                "{} is a symlink or not a regular file, so it is not a ticket in this store",
                path.display()
            ),
            Self::StoreUnreadable(path, source) => write!(
                f,
                "cannot read the ticket store at {}: {source}",
                path.display()
            ),
            Self::ReadSchema(path, source) => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::ParseSchema(path, source) => {
                write!(f, "failed to parse {}: {source}", path.display())
            }
            // Inventing a schema would make every store agree with itself by construction, which is
            // the opposite of what a declared contract is for.
            Self::SchemaMissing(path) => write!(
                f,
                "this store has no schema, so its frontmatter contract is undeclared.\n\
                 Create {} declaring the valid keys, which are required, and each enum's \
                 permitted values.",
                path.display()
            ),
            Self::ReadTicket(path, source) => {
                write!(f, "failed to read ticket {}: {source}", path.display())
            }
            Self::TicketIdNotAName(id) => write!(
                f,
                "`{id}` is not a ticket id; an id is a bare filename within the store, \
                 with no path separators"
            ),
            Self::UnknownTicket { id, candidates } => {
                write!(f, "no ticket `{id}` in the store")?;
                if candidates.is_empty() {
                    return Ok(());
                }
                write!(f, "; did you mean one of:")?;
                for candidate in candidates {
                    write!(f, "\n  {candidate}")?;
                }
                Ok(())
            }
            Self::UnknownSection {
                id,
                section,
                available,
            } => {
                write!(f, "ticket `{id}` has no section `{section}`")?;
                if available.is_empty() {
                    return Ok(());
                }
                write!(f, "; it has:")?;
                for name in available {
                    write!(f, "\n  {name}")?;
                }
                Ok(())
            }
            Self::UndeclaredFilterKey { key, declared } => write!(
                f,
                "cannot filter on `{key}`: this store's schema does not declare it. Declared keys: {}",
                match declared.is_empty() {
                    true => "<none>".to_string(),
                    false => declared.join(", "),
                }
            ),
        }
    }
}

impl std::error::Error for SkaldError {}

impl From<io::Error> for SkaldError {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

impl From<serde_json::Error> for SkaldError {
    fn from(source: serde_json::Error) -> Self {
        Self::SerializeJson(source)
    }
}
