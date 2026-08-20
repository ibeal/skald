use clap::{ArgAction, Parser, Subcommand};

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
}
