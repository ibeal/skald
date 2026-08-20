//! `skald list` — querying the store on the compiled-in fields.

use serde_json::{Value as Json, json};

use crate::contract::Field;
use crate::errors::{Result, SkaldError};
use crate::show::frontmatter_json;
use crate::store::{Listing, Store};
use crate::ticket::Ticket;

/// Marks a parked row. `status` alone no longer says a ticket is paused, because the pause reason is
/// a separate field so that pausing never loses the phase.
const PAUSED_MARKER: &str = "!";

#[derive(Debug, Default)]
pub struct Filters {
    pub status: Option<String>,
    pub repo: Option<String>,
    pub parent: Option<String>,
    /// Keep only parked tickets.
    pub paused: bool,
}

pub fn select(store: &Store, filters: &Filters) -> Result<Listing> {
    // A bad status is caught before the store is read, so a typo names the permitted set rather than
    // silently matching nothing and looking like an empty store.
    if let Some(status) = &filters.status {
        crate::contract::Status::parse(status).ok_or_else(|| SkaldError::UnknownStatus {
            value: status.clone(),
            permitted: crate::contract::STATUSES
                .iter()
                .map(|status| status.name().to_string())
                .collect(),
        })?;
    }

    let mut listing = store.tickets()?;
    listing.tickets.retain(|ticket| matches(ticket, filters));
    Ok(listing)
}

fn matches(ticket: &Ticket, filters: &Filters) -> bool {
    if let Some(status) = &filters.status
        && ticket.scalar(Field::Status.key()) != Some(status.as_str())
    {
        return false;
    }
    if let Some(repo) = &filters.repo
        && !ticket.list(Field::Repos.key()).contains(&repo.as_str())
    {
        return false;
    }
    if let Some(parent) = &filters.parent
        && ticket.scalar(Field::Parent.key()) != Some(parent.as_str())
    {
        return false;
    }
    if filters.paused && !is_paused(ticket) {
        return false;
    }
    true
}

/// Presence, not a value: any reason means parked.
fn is_paused(ticket: &Ticket) -> bool {
    ticket.scalar(Field::Paused.key()).is_some()
}

pub fn render_json(tickets: &[Ticket]) -> Json {
    Json::Array(
        tickets
            .iter()
            .map(|ticket| {
                json!({
                    "id": ticket.id,
                    "path": ticket.path.display().to_string(),
                    "frontmatter": frontmatter_json(ticket),
                })
            })
            .collect(),
    )
}

/// A compact aligned table: `ID · STATUS · TITLE · UPDATED`.
///
/// Empty when nothing matched, so the output composes with a pipeline rather than making a caller
/// strip a "no results" line.
pub fn render_table(tickets: &[Ticket]) -> String {
    if tickets.is_empty() {
        return String::new();
    }

    let header = ["ID", "STATUS", "TITLE", "UPDATED"].map(str::to_string);
    let rows: Vec<[String; 4]> = tickets.iter().map(cells).collect();
    let widths: [usize; 4] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .chain(std::iter::once(header[column].chars().count()))
            .max()
            .unwrap_or_default()
    });

    let mut out = String::new();
    for row in std::iter::once(&header).chain(rows.iter()) {
        out.push_str(&pad(row, &widths));
        out.push('\n');
    }
    if tickets.iter().any(is_paused) {
        out.push_str("\n! = paused; `skald show <id>` for the reason\n");
    }
    out
}

fn cells(ticket: &Ticket) -> [String; 4] {
    let status = match ticket.scalar(Field::Status.key()) {
        Some(status) if is_paused(ticket) => format!("{status} {PAUSED_MARKER}"),
        Some(status) => status.to_string(),
        None => String::new(),
    };
    [
        ticket.id.clone(),
        status,
        ticket
            .scalar(Field::Title.key())
            .unwrap_or_default()
            .to_string(),
        ticket
            .scalar(Field::Updated.key())
            .unwrap_or_default()
            .to_string(),
    ]
}

fn pad(row: &[String; 4], widths: &[usize; 4]) -> String {
    row.iter()
        .zip(widths)
        .map(|(cell, width)| format!("{cell:<width$}"))
        .collect::<Vec<_>>()
        .join("  ")
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{Filters, render_table, select};
    use crate::errors::SkaldError;
    use crate::store::{Store, fixture};

    const ONE: &str = "---\ntitle: Bootstrap and the read side\nstatus: building\nrepos:\n  - skald\nparent: parent-ticket\nupdated: 2026-08-20\n---\n\n## Log\n";
    const TWO: &str = "---\ntitle: Validate a store\nstatus: refining\npaused: waiting on the schema decision\nrepos:\n  - skald\n  - dotfiles\nparent: parent-ticket\nupdated: 2026-08-19\n---\n\n## Log\n";
    const THREE: &str = "---\ntitle: Unrelated\nstatus: done\nrepos:\n  - other\nupdated: 2026-08-18\n---\n\n## Log\n";

    fn store() -> (std::path::PathBuf, Store) {
        let root = fixture(
            "list",
            &[("one.md", ONE), ("two.md", TWO), ("three.md", THREE)],
        );
        let store = Store::at(root.clone()).unwrap();
        (root, store)
    }

    fn ids(store: &Store, filters: &Filters) -> Vec<String> {
        select(store, filters)
            .unwrap()
            .tickets
            .into_iter()
            .map(|ticket| ticket.id)
            .collect()
    }

    #[test]
    fn each_filter_narrows_on_its_own_field() {
        let (root, store) = store();
        assert_eq!(
            ids(&store, &Filters::default()),
            vec!["one", "three", "two"]
        );
        assert_eq!(
            ids(
                &store,
                &Filters {
                    status: Some("building".into()),
                    ..Filters::default()
                }
            ),
            vec!["one"]
        );
        // A list field matches on membership.
        assert_eq!(
            ids(
                &store,
                &Filters {
                    repo: Some("dotfiles".into()),
                    ..Filters::default()
                }
            ),
            vec!["two"]
        );
        assert_eq!(
            ids(
                &store,
                &Filters {
                    parent: Some("parent-ticket".into()),
                    ..Filters::default()
                }
            ),
            vec!["one", "two"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn paused_filters_on_presence_not_on_a_value() {
        let (root, store) = store();
        assert_eq!(
            ids(
                &store,
                &Filters {
                    paused: true,
                    ..Filters::default()
                }
            ),
            vec!["two"]
        );
        // Filters compose, and a paused ticket keeps its phase — so it is still found by it.
        assert_eq!(
            ids(
                &store,
                &Filters {
                    status: Some("refining".into()),
                    paused: true,
                    ..Filters::default()
                }
            ),
            vec!["two"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_status_that_is_not_in_the_enum_is_refused_before_the_store_is_read() {
        let (root, store) = store();
        // Silently matching nothing would read as "an empty store", which is the wrong diagnosis.
        assert!(matches!(
            select(
                &store,
                &Filters {
                    status: Some("build".into()),
                    ..Filters::default()
                }
            ),
            Err(SkaldError::UnknownStatus { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_table_carries_the_title_and_marks_a_parked_row() {
        let (root, store) = store();
        let tickets = select(&store, &Filters::default()).unwrap().tickets;
        assert_eq!(
            render_table(&tickets),
            "ID     STATUS      TITLE                        UPDATED\n\
             one    building    Bootstrap and the read side  2026-08-20\n\
             three  done        Unrelated                    2026-08-18\n\
             two    refining !  Validate a store             2026-08-19\n\
             \n! = paused; `skald show <id>` for the reason\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_legend_is_absent_when_nothing_is_parked() {
        let (root, store) = store();
        let tickets = select(
            &store,
            &Filters {
                status: Some("building".into()),
                ..Filters::default()
            },
        )
        .unwrap()
        .tickets;
        assert!(!render_table(&tickets).contains("paused"));
        // Nothing matched means no output, so the result composes in a pipeline.
        assert_eq!(render_table(&[]), "");
        std::fs::remove_dir_all(root).unwrap();
    }
}
