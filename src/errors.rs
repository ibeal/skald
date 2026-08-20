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
    ReadTicket(PathBuf, io::Error),
    WriteTicket(PathBuf, io::Error),
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
    UnknownStatus {
        value: String,
        permitted: Vec<String>,
    },
    TicketExists(PathBuf),
    TerminalStatus {
        current: &'static str,
        next: &'static str,
    },
    CriteriaFrozen {
        id: String,
        status: String,
    },
    MultiLineValue(&'static str),
    NewlineInValue(&'static str),
    EmptyText(&'static str),
    WouldViolate(Vec<String>),
    NoText,
    TextTwice,
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
            Self::ReadTicket(path, source) => {
                write!(f, "failed to read ticket {}: {source}", path.display())
            }
            Self::WriteTicket(path, source) => {
                write!(f, "failed to write ticket {}: {source}", path.display())
            }
            Self::TicketIdNotAName(id) => write!(
                f,
                "`{id}` is not a ticket id. An id is a bare name within the store: no path \
                 separators, no leading dot, and at most one trailing `.md`"
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
            // Naming the permitted set matters more here than anywhere else: this is the enum whose
            // decoration motivated the tool, so the message has to make the bare value obvious and
            // say where the qualifier goes instead.
            Self::UnknownStatus { value, permitted } => write!(
                f,
                "`{value}` is not a status; it is one of: {}\n\
                 A status takes one bare value. If the state needs explaining, set the bare value \
                 and put the explanation in the log:\n\
                 \x20 skald log <id> \"...\"",
                permitted.join(", ")
            ),
            Self::TicketExists(path) => write!(
                f,
                "{} already exists; refusing to overwrite it",
                path.display()
            ),
            Self::TerminalStatus { current, next } => write!(
                f,
                "`{current}` is terminal, so it cannot move to `{next}`.\n\
                 Follow-up work is a new ticket pointing back at this one:\n\
                 \x20 skald new <id> --parent <this-id>"
            ),
            // The error has to name the way through, because there deliberately is no flag for it:
            // changing the criteria *is* a return to refining, and saying so leaves a trace that an
            // override flag would not.
            Self::CriteriaFrozen { id, status } => write!(
                f,
                "the acceptance criteria are frozen outside refining (status: {status}).\n\
                 A deliberate change is a return to refining:\n\
                 \x20 skald log {id} \"what you found\"\n\
                 \x20 skald set {id} --status refining"
            ),
            Self::TextTwice => write!(
                f,
                "text was given both as an argument and with --stdin; pass exactly one"
            ),
            Self::NoText => write!(
                f,
                "no text given; pass it as an argument or read it from standard input with --stdin"
            ),
            Self::NewlineInValue(key) => write!(
                f,
                "`{key}` may not contain a line break; a frontmatter value is one line.\n\
                 Prose belongs in the log:\n\
                 \x20 skald log <id> \"...\""
            ),
            Self::EmptyText(what) => write!(
                f,
                "refusing to write empty {what}. If this was `--stdin`, the redirect read nothing"
            ),
            Self::MultiLineValue(key) => write!(
                f,
                "`{key}` spans more than one line, so a single value cannot replace it"
            ),
            Self::WouldViolate(violations) => {
                write!(
                    f,
                    "refusing to write: the result would not pass `skald check`"
                )?;
                for violation in violations {
                    write!(f, "\n  {violation}")?;
                }
                Ok(())
            }
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
