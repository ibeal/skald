mod check;
mod cli;
mod config;
mod contract;
mod docs;
mod errors;
mod list;
mod show;
mod store;
mod ticket;
mod webhook;
mod write;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::errors::Result;
use crate::store::Store;

/// `check` exits with this when the store is invalid, so a hook or CI step can tell "the contract is
/// broken" apart from "skald could not run" — which exits 1.
const INVALID: u8 = 2;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    // `docs` resolves no store on purpose. It is the recovery path for an agent that cannot work out
    // the interface, and "the store is not configured" is one of the things it explains — so it must
    // not fail on the very condition it exists to describe.
    if let Commands::Docs { topic } = cli.command {
        print!("{}", docs::render(topic));
        return Ok(ExitCode::SUCCESS);
    }

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
            status,
            repo,
            parent,
            paused,
            json,
        } => {
            let listing = list::select(
                &store,
                &list::Filters {
                    status,
                    repo,
                    parent,
                    paused,
                },
            )?;
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
        Commands::Check { fix } => return run_check(&store, fix),
        Commands::New { id, fields } => {
            let created = write::new(&store, &id, &changes(fields))?;
            println!("created {created}");
        }
        Commands::Set { id, fields } => {
            let ticket = store.ticket(&id)?;
            let previous_status = ticket.scalar("status").unwrap_or_default().to_string();
            let previous_paused = ticket.scalar("paused");
            let next_status = fields.status.as_deref().unwrap_or(&previous_status);
            let next_paused = fields.paused.as_deref().or(previous_paused);
            let previous_derived = webhook::derived_status(&previous_status, previous_paused);
            let next_derived = webhook::derived_status(next_status, next_paused);
            let applied = write::set(&store, &ticket, &changes(fields))?;
            match applied.is_empty() {
                // Silence would read as success, and a caller that mistyped a flag deserves to know
                // nothing happened.
                true => println!("nothing to change"),
                false => {
                    for line in applied {
                        println!("{line}");
                    }
                }
            }
            if previous_derived != next_derived {
                let updated = store.ticket(&id)?;
                webhook::emit(
                    &updated,
                    &previous_derived,
                    &next_derived,
                    store.webhook_url.as_deref(),
                );
            }
        }
        Commands::Ac { id, text, stdin } => {
            let ticket = store.ticket(&id)?;
            write::set_criteria(&store, &ticket, &text_arg(text, stdin)?)?;
        }
        Commands::Log { id, text, stdin } => {
            let ticket = store.ticket(&id)?;
            write::log(&store, &ticket, &text_arg(text, stdin)?)?;
        }
        // Handled above, before the store is resolved.
        Commands::Docs { .. } => unreachable!("docs returns before the store is resolved"),
    }

    Ok(ExitCode::SUCCESS)
}

fn changes(fields: cli::Fields) -> write::Changes {
    write::Changes {
        title: fields.title,
        status: fields.status,
        paused: fields.paused,
        repos: fields.repos,
        branch: fields.branch,
        link: fields.link,
        pr: fields.pr,
        parent: fields.parent,
    }
}

/// Body text from an argument or from stdin. `--stdin` exists because multi-line content through
/// shell quoting is how an agent gets it wrong.
fn text_arg(text: Option<String>, stdin: bool) -> Result<String> {
    match (text, stdin) {
        // Silently preferring one over the other would discard whichever the caller meant.
        (Some(_), true) => Err(errors::SkaldError::TextTwice),
        (None, true) => {
            let mut buffer = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)?;
            Ok(buffer)
        }
        (Some(text), false) => Ok(text),
        (None, false) => Err(errors::SkaldError::NoText),
    }
}

fn run_check(store: &Store, fix: bool) -> Result<ExitCode> {
    if fix {
        for ticket in &store.tickets()?.tickets {
            if let Some(fixed) = check::fix(store, ticket)? {
                for change in &fixed.changes {
                    println!("{}: {change}", fixed.path.display());
                }
            }
        }
    }

    // Re-read after fixing, so what is reported is the state on disk rather than the state before.
    // A `--fix` run still exits non-zero when anything is left, which is the point: the repairs are
    // shape, and what remains needs a human.
    let violations = check::run(store)?;
    for violation in &violations {
        println!("{}", violation.render());
    }
    match violations.is_empty() {
        // Silence on success, so this composes in a pre-commit hook.
        true => Ok(ExitCode::SUCCESS),
        false => Ok(ExitCode::from(INVALID)),
    }
}
