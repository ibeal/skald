use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum DocsTopic {
    /// How to drive skald from inside an agent session.
    Agent,
}

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " ", env!("SKALD_GIT_SHA"));

#[derive(Debug, Parser)]
#[command(name = "skald")]
#[command(about = "Read and write the tickets in $SKALD_STORE")]
#[command(version = VERSION, disable_version_flag = true)]
pub struct Cli {
    /// Print the version and Git SHA.
    #[arg(
        short = 'v',
        long = "version",
        action = ArgAction::Version,
        required = false
    )]
    _version: Option<bool>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print a whole ticket, or one of its sections.
    Show {
        /// Ticket id, with or without the `.md` extension.
        id: String,
        /// Print only this section. Matched loosely: `Build log`, `build-log`, and `BUILD LOG` are
        /// the same section.
        #[arg(long)]
        section: Option<String>,
        /// Emit JSON instead of the file's own bytes.
        #[arg(long)]
        json: bool,
    },
    /// List the tickets in the store, optionally filtered.
    List {
        /// Keep only tickets in this status.
        #[arg(long)]
        status: Option<String>,
        /// Keep only tickets whose `repos` include this.
        #[arg(long)]
        repo: Option<String>,
        /// Keep only tickets split from or following up this one.
        #[arg(long)]
        parent: Option<String>,
        /// Keep only parked tickets.
        #[arg(long)]
        paused: bool,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Validate every ticket in the store. Exits non-zero on any violation.
    Check {
        /// Repair what is unambiguously mechanical. Never guesses at a status.
        #[arg(long)]
        fix: bool,
    },
    /// Create a ticket. Refuses to overwrite one that exists.
    New {
        /// Ticket id; becomes the filename.
        id: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Change a ticket's fields. Several at once land in one write.
    Set {
        id: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Replace the acceptance criteria. Only while the ticket is refining.
    Ac {
        id: String,
        /// The new criteria. Omit and pass `--stdin` for multi-line content.
        ///
        /// Hyphens are allowed: criteria are written as `- a bullet`, so treating a leading `-` as a
        /// flag would reject the most ordinary input there is.
        #[arg(allow_hyphen_values = true)]
        text: Option<String>,
        /// Read the criteria from standard input.
        #[arg(long)]
        stdin: bool,
    },
    /// Print built-in guidance.
    Docs {
        #[arg(value_enum, default_value_t = DocsTopic::Agent)]
        topic: DocsTopic,
    },
    /// Append a dated entry to the log.
    Log {
        id: String,
        /// The entry. Omit and pass `--stdin` for multi-line content.
        #[arg(allow_hyphen_values = true)]
        text: Option<String>,
        /// Read the entry from standard input.
        #[arg(long)]
        stdin: bool,
    },
}

/// The writable fields, shared by `new` and `set`.
///
/// One flag per field rather than a `<key> <value>` pair: the field set is closed, so there is no
/// reason to pass a key as data, and flags give shell completion plus several fields in one atomic
/// write with a single `updated` bump.
#[derive(Debug, clap::Args)]
pub struct Fields {
    /// What this ticket is, in one line.
    #[arg(long)]
    pub title: Option<String>,
    /// refining | building | reviewing | done | cancelled
    #[arg(long)]
    pub status: Option<String>,
    /// Why the ticket should not be worked on. Pass "" to resume.
    #[arg(long)]
    pub paused: Option<String>,
    /// Repo this work touches. Repeat for several; pass "" once to empty the list.
    #[arg(long = "repo")]
    pub repos: Option<Vec<String>>,
    /// Where the code lives.
    #[arg(long)]
    pub branch: Option<String>,
    /// The upstream ticket or issue.
    #[arg(long)]
    pub link: Option<String>,
    /// The pull request.
    #[arg(long)]
    pub pr: Option<String>,
    /// The ticket this one was split from or follows up.
    #[arg(long)]
    pub parent: Option<String>,
}
