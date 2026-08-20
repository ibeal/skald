mod cli;
mod errors;
mod list;
mod schema;
mod show;
mod store;
mod ticket;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::errors::Result;
use crate::store::Store;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::resolve()?;

    match cli.command {
        Commands::Show { id, section, json } => {
            let ticket = store.ticket(&id)?;
            if let Some(warning) = show::frontmatter_warning(&ticket) {
                eprintln!("{warning}");
            }
            match json {
                true => println!(
                    "{}",
                    serde_json::to_string_pretty(&show::render_json(&ticket, section.as_deref())?)?
                ),
                // `print!`, not `println!`: the ticket's own trailing newline is part of the file,
                // and adding one would make `show` stop reproducing it exactly.
                false => print!("{}", show::render_text(&ticket, section.as_deref())?),
            }
        }
        Commands::List {
            phase,
            project,
            json,
        } => {
            let listing = list::select(&store, &list::Filters { phase, project })?;
            // A file that could not be read is named on stderr and the rest of the listing still
            // prints. `list` reports; `check` is what fails.
            for error in &listing.unreadable {
                eprintln!("warning: {error}");
            }
            match json {
                true => println!(
                    "{}",
                    serde_json::to_string_pretty(&list::render_json(&listing.tickets))?
                ),
                false => print!("{}", list::render_table(&listing.tickets)),
            }
        }
    }

    Ok(())
}
