//! `skald list` — querying the store on declared frontmatter keys.

use serde_json::{Value as Json, json};

use crate::errors::Result;
use crate::show::frontmatter_json;
use crate::store::{Listing, Store};
use crate::ticket::Ticket;

/// The frontmatter key holding a ticket's phase. Named once so the filter, the column, and any
/// future caller cannot drift apart.
const PHASE_KEY: &str = "phase";
const PROJECTS_KEY: &str = "projects";
const UPDATED_KEY: &str = "updated";

/// Columns in the default table. These names are a store convention rather than something skald
/// owns, so a store that does not declare one renders it blank instead of failing — `--json` is the
/// complete data either way.
const COLUMNS: [&str; 4] = ["id", PHASE_KEY, UPDATED_KEY, PROJECTS_KEY];

#[derive(Debug, Default)]
pub struct Filters {
    pub phase: Option<String>,
    pub project: Option<String>,
}

pub fn select(store: &Store, filters: &Filters) -> Result<Listing> {
    if filters.phase.is_some() {
        store.check_filter_key(PHASE_KEY)?;
    }
    if filters.project.is_some() {
        store.check_filter_key(PROJECTS_KEY)?;
    }

    let mut listing = store.tickets()?;
    listing.tickets.retain(|ticket| matches(ticket, filters));
    Ok(listing)
}

fn matches(ticket: &Ticket, filters: &Filters) -> bool {
    if let Some(phase) = &filters.phase
        && ticket.scalar(PHASE_KEY) != Some(phase.as_str())
    {
        return false;
    }
    if let Some(project) = &filters.project
        && !ticket.list(PROJECTS_KEY).contains(&project.as_str())
    {
        return false;
    }
    true
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

/// A compact aligned table. Empty when nothing matched, so the output composes with a pipeline
/// rather than making a caller strip a "no results" line.
pub fn render_table(tickets: &[Ticket]) -> String {
    if tickets.is_empty() {
        return String::new();
    }

    let rows: Vec<Vec<String>> = tickets.iter().map(cells).collect();
    let widths: Vec<usize> = COLUMNS
        .iter()
        .enumerate()
        .map(|(column, name)| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .chain(std::iter::once(name.chars().count()))
                .max()
                .unwrap_or_default()
        })
        .collect();

    let header: Vec<String> = COLUMNS.iter().map(|name| name.to_uppercase()).collect();
    let mut out = String::new();
    for row in std::iter::once(&header).chain(rows.iter()) {
        out.push_str(&pad(row, &widths));
        out.push('\n');
    }
    out
}

fn cells(ticket: &Ticket) -> Vec<String> {
    COLUMNS
        .iter()
        .map(|column| match *column {
            "id" => ticket.id.clone(),
            PROJECTS_KEY => ticket.list(PROJECTS_KEY).join(", "),
            key => ticket.scalar(key).unwrap_or_default().to_string(),
        })
        .collect()
}

fn pad(row: &[String], widths: &[usize]) -> String {
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

    const SCHEMA: &str = "\
[keys.id]
type = \"string\"
[keys.phase]
type = \"enum\"
values = [\"build\", \"review\"]
[keys.updated]
type = \"date\"
[keys.projects]
type = \"list\"
";

    const ONE: &str =
        "---\nid: one\nphase: build\nupdated: 2026-08-20\nprojects:\n  - skald\n---\n\n# One\n";
    const TWO: &str = "---\nid: two\nphase: review\nupdated: 2026-08-19\nprojects:\n  - skald\n  - dotfiles\n---\n\n# Two\n";

    fn store() -> (std::path::PathBuf, Store) {
        let root = fixture(
            "list",
            &[("one.md", ONE), ("two.md", TWO), (".schema.toml", SCHEMA)],
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
    fn filters_narrow_on_scalars_and_on_list_membership() {
        let (root, store) = store();
        assert_eq!(ids(&store, &Filters::default()), vec!["one", "two"]);
        assert_eq!(
            ids(
                &store,
                &Filters {
                    phase: Some("build".into()),
                    ..Filters::default()
                }
            ),
            vec!["one"]
        );
        assert_eq!(
            ids(
                &store,
                &Filters {
                    project: Some("dotfiles".into()),
                    ..Filters::default()
                }
            ),
            vec!["two"]
        );
        // Filters compose, and a phase nothing carries matches nothing rather than everything.
        assert!(
            ids(
                &store,
                &Filters {
                    phase: Some("merged".into()),
                    project: Some("skald".into()),
                }
            )
            .is_empty()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_table_aligns_and_names_its_columns() {
        let (root, store) = store();
        let tickets = select(&store, &Filters::default()).unwrap().tickets;
        assert_eq!(
            render_table(&tickets),
            "ID   PHASE   UPDATED     PROJECTS\n\
             one  build   2026-08-20  skald\n\
             two  review  2026-08-19  skald, dotfiles\n"
        );
        // Nothing matched means no output, so the result composes in a pipeline.
        assert_eq!(render_table(&[]), "");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_store_that_does_not_declare_a_filter_key_refuses_the_filter() {
        let root = fixture(
            "list-nophase",
            &[
                ("one.md", ONE),
                (".schema.toml", "[keys.id]\ntype = \"string\"\n"),
            ],
        );
        let store = Store::at(root.clone()).unwrap();
        assert!(matches!(
            select(
                &store,
                &Filters {
                    phase: Some("build".into()),
                    ..Filters::default()
                }
            ),
            Err(SkaldError::UndeclaredFilterKey { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
