//! The write side: `new`, `set`, `ac`, and `log`.
//!
//! Four commands, because the ticket shape is closed — frontmatter, acceptance criteria, and log are
//! all a ticket has, and each has exactly one writer. Once the permission deny lands these are the
//! only way an agent can write a ticket at all, so the rules they enforce are the rules, full stop.
//!
//! Every mutation goes through [`crate::ticket::Edits`], so the rest of the file survives
//! byte-for-byte. And every mutation is checked against the contract before it lands: skald must not
//! be able to create a violation it would later report.

use crate::check;
use crate::contract::{FIELDS, Field, Owned, STATUSES, Status};
use crate::errors::{Result, SkaldError};
use crate::store::Store;
use crate::ticket::{Edits, Ticket};

/// Today, in the local timezone.
///
/// Local rather than UTC on purpose: these dates are a human's note of a day, and a log entry written
/// at 6pm dated tomorrow is a small lie that compounds.
pub fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

/// The fields a `set` or a `new` is asking to change. `None` means "leave alone"; `Some("")` means
/// "clear", which is how every field is emptied uniformly.
#[derive(Debug, Default)]
pub struct Changes {
    pub title: Option<String>,
    pub status: Option<String>,
    pub paused: Option<String>,
    /// Replaces the whole list. A single empty string empties it.
    pub repos: Option<Vec<String>>,
    pub branch: Option<String>,
    pub link: Option<String>,
    pub pr: Option<String>,
    pub parent: Option<String>,
}

impl Changes {
    fn pairs(&self) -> Vec<(Field, String)> {
        let mut pairs = Vec::new();
        let mut push = |field: Field, value: &Option<String>| {
            if let Some(value) = value {
                pairs.push((field, value.clone()));
            }
        };
        push(Field::Title, &self.title);
        push(Field::Status, &self.status);
        push(Field::Paused, &self.paused);
        push(Field::Branch, &self.branch);
        push(Field::Link, &self.link);
        push(Field::Pr, &self.pr);
        push(Field::Parent, &self.parent);
        pairs
    }

    fn is_empty(&self) -> bool {
        self.pairs().is_empty() && self.repos.is_none()
    }
}

/// Scaffold a ticket. Refuses to overwrite one that exists.
///
/// There is no template file: the shape is compiled in, so there is nothing to keep in sync and no
/// way for a store to drift from the contract it is validated against.
pub fn new(store: &Store, id: &str, changes: &Changes) -> Result<String> {
    validate_title(changes.title.as_deref())?;
    // One normalization, shared with every read path, so the id reported back always resolves to the
    // file that was written.
    let id = store.normalized_id(id)?;
    let path = store.path_for(&id)?;
    if path.exists() {
        return Err(SkaldError::TicketExists(path));
    }

    let today = today();
    let mut body = String::from("---\n");
    for field in FIELDS {
        let value = match field {
            Field::Status => changes
                .status
                .clone()
                .unwrap_or_else(|| Status::Refining.name().to_string()),
            Field::Created | Field::Updated => today.clone(),
            Field::Repos => String::new(),
            _ => changes
                .pairs()
                .iter()
                .find(|(candidate, _)| *candidate == field)
                .map(|(_, value)| value.clone())
                .unwrap_or_default(),
        };
        match (field, value.is_empty()) {
            (Field::Repos, _) => {
                body.push_str("repos:\n");
                for repo in changes.repos.iter().flatten() {
                    body.push_str(&format!("  - {repo}\n"));
                }
            }
            (_, true) => body.push_str(&format!("{}:\n", field.key())),
            (_, false) => body.push_str(&format!("{}: {value}\n", field.key())),
        }
    }
    body.push_str("---\n");
    for owned in [Owned::AcceptanceCriteria, Owned::Log] {
        body.push_str(&format!("\n## {}\n", owned.heading()));
    }

    let ticket = Ticket::parse(id, &path, body);
    guard(store, None, &ticket)?;
    commit(&ticket)?;
    Ok(ticket.id)
}

/// Apply field changes in one atomic write, with one `updated` bump and — when the status moved — one
/// log line.
pub fn set(store: &Store, ticket: &Ticket, changes: &Changes) -> Result<Vec<String>> {
    if changes.is_empty() {
        return Ok(Vec::new());
    }

    let mut applied = Vec::new();
    let mut edits = ticket.edits();

    let previous = ticket.scalar(Field::Status.key()).map(str::to_string);
    if let Some(next) = &changes.status {
        validate_status(previous.as_deref(), next)?;
    }

    for (field, value) in changes.pairs() {
        if field == Field::Status
            && let Some(previous) = &previous
            && previous == &value
        {
            // A no-op set should not leave a log line claiming a move.
            continue;
        }
        write_field(ticket, &mut edits, field, &value)?;
        applied.push(match value.is_empty() {
            true => format!("cleared `{}`", field.key()),
            false => format!("set `{}` to {value}", field.key()),
        });
    }
    if let Some(repos) = &changes.repos {
        write_list(ticket, &mut edits, Field::Repos, repos)?;
        applied.push(format!("set `repos` to {}", repos.join(", ")));
    }

    if applied.is_empty() {
        return Ok(applied);
    }

    // One clock read for the whole command, so a `set` that straddles midnight cannot date its log
    // line and its `updated` differently.
    let today = today();

    // Only a status change is logged. Logging every mutation was considered and rejected: `set pr`
    // lines would dilute the resume surface that is the log's whole purpose.
    if let Some(next) = &changes.status
        && previous.as_deref() != Some(next.as_str())
    {
        let from = previous.as_deref().unwrap_or("(unset)");
        append_entry(
            ticket,
            &mut edits,
            &today,
            &format!("status {from} → {next}"),
        );
    }

    write_field(ticket, &mut edits, Field::Updated, &today)?;

    let rendered = edits.render();
    let updated = Ticket::parse(ticket.id.clone(), &ticket.path, rendered);
    guard(store, Some(ticket), &updated)?;
    commit(&updated)?;
    Ok(applied)
}

/// Replace the acceptance criteria wholesale.
///
/// Refused unless the ticket is refining. There is deliberately no override: changing the criteria
/// *is* a return to refining, so the way through is to say so, which leaves a trace in the status
/// history and the log. An override flag would become muscle memory and leave none.
pub fn set_criteria(store: &Store, ticket: &Ticket, text: &str) -> Result<()> {
    let status = ticket.scalar(Field::Status.key()).unwrap_or_default();
    if Status::parse(status) != Some(Status::Refining) {
        return Err(SkaldError::CriteriaFrozen {
            id: ticket.id.clone(),
            status: status.to_string(),
        });
    }
    // The freeze exists to protect this content, so erasing it must not be the one thing that slips
    // through. An empty `--stdin` is a redirect that read nothing, not an intention.
    if text.trim().is_empty() {
        return Err(SkaldError::EmptyText("acceptance criteria"));
    }

    let mut edits = ticket.edits();
    match ticket.section(Owned::AcceptanceCriteria.heading()) {
        Some(section) => {
            // A section body owns the blank line that separates it from the next heading, so a
            // replacement has to put it back or the log's heading ends up flush against the criteria.
            let separator = match ticket.section_is_followed(section) {
                true => "\n\n",
                false => "\n",
            };
            let body = format!("\n{}{separator}", text.trim_end());
            ticket.set_section_edit(&mut edits, section, &body);
        }
        // A migrated ticket may not have the section yet; an agent should not have to know.
        None => insert_criteria_section(ticket, &mut edits, text.trim_end()),
    }
    write_field(ticket, &mut edits, Field::Updated, &today())?;

    let rendered = edits.render();
    let updated = Ticket::parse(ticket.id.clone(), &ticket.path, rendered);
    guard(store, Some(ticket), &updated)?;
    commit(&updated)
}

/// Append a dated bullet to the log.
///
/// The log is the only append-only thing in a ticket, and nothing can replace it: rewriting an audit
/// trail destroys the property that makes it worth reading on resume.
pub fn log(store: &Store, ticket: &Ticket, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(SkaldError::EmptyText("log entry"));
    }
    let today = today();
    let mut edits = ticket.edits();
    append_entry(ticket, &mut edits, &today, text);
    write_field(ticket, &mut edits, Field::Updated, &today)?;

    let rendered = edits.render();
    let updated = Ticket::parse(ticket.id.clone(), &ticket.path, rendered);
    guard(store, Some(ticket), &updated)?;
    commit(&updated)
}

fn append_entry<'a>(ticket: &'a Ticket, edits: &mut Edits<'a>, today: &str, text: &str) {
    // `log` writes the bullet, so a caller that wrote one too gets `- 2026-08-20: - text`. Stripping
    // one leading marker is not a guess: the entry's own list marker is redundant by construction.
    let text = text.trim();
    let text = text
        .strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .unwrap_or(text);
    let entry = format!("- {today}: {}", text.trim_start());
    match ticket.section(Owned::Log.heading()) {
        Some(section) => ticket.append_to_section_edit(edits, section, &entry),
        None => {
            let newline = ticket.newline();
            append_section(
                ticket,
                edits,
                Owned::Log,
                &format!("{newline}{entry}{newline}"),
            );
        }
    }
}

/// Add the acceptance criteria to a ticket that has none, **above** the log where one exists.
///
/// Appending would put the contract after the running commentary, which is contract-clean but reads
/// backwards — and a migrated ticket is exactly the one a human is about to read.
fn insert_criteria_section<'a>(ticket: &'a Ticket, edits: &mut Edits<'a>, text: &str) {
    let newline = ticket.newline();
    let section = format!(
        "## {}{newline}{newline}{text}{newline}",
        Owned::AcceptanceCriteria.heading()
    );
    match ticket.section(Owned::Log.heading()) {
        Some(log) => {
            let at = ticket.section_heading_range(log).start;
            edits.replace(at..at, format!("{section}{newline}"));
        }
        None => append_section(
            ticket,
            edits,
            Owned::AcceptanceCriteria,
            &format!("{newline}{text}{newline}"),
        ),
    }
}

fn append_section<'a>(ticket: &'a Ticket, edits: &mut Edits<'a>, owned: Owned, body: &str) {
    let end = ticket.source().len();
    let separator = match ticket.source().ends_with('\n') {
        true => "",
        false => "\n",
    };
    edits.replace(
        end..end,
        format!("{separator}\n## {}\n{body}", owned.heading()),
    );
}

fn write_field<'a>(
    ticket: &'a Ticket,
    edits: &mut Edits<'a>,
    field: Field,
    value: &str,
) -> Result<()> {
    if field == Field::Title {
        validate_title(Some(value))?;
    }
    // A newline in a value would end the `key: value` line and leave a bare line in the block, which
    // is neither a key nor a continuation: the parser ignores it, `check` sees nothing wrong, and the
    // frontmatter is quietly not YAML any more.
    if value.contains(['\n', '\r']) {
        return Err(SkaldError::NewlineInValue(field.key()));
    }

    match ticket.entry(field.key()) {
        Some(entry) => {
            if ticket.entry_text(entry).trim_end().contains('\n') {
                return Err(SkaldError::MultiLineValue(field.key()));
            }
            let text = match value.is_empty() {
                true => String::new(),
                false => format!(" {value}"),
            };
            edits.replace(entry.value_range(), text);
        }
        None => {
            let rendered = match value.is_empty() {
                true => format!("{}:\n", field.key()),
                false => format!("{}: {value}\n", field.key()),
            };
            edits.replace(insert_position(ticket, field), rendered);
        }
    }
    Ok(())
}

/// Title colons are refused on writes without making historical stores invalid. A legacy ticket
/// remains readable and checkable; changing its title is the explicit migration boundary.
fn validate_title(value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| value.contains(':')) {
        return Err(SkaldError::TitleContainsColon);
    }
    Ok(())
}

fn write_list<'a>(
    ticket: &'a Ticket,
    edits: &mut Edits<'a>,
    field: Field,
    items: &[String],
) -> Result<()> {
    let mut rendered = format!("{}:\n", field.key());
    for item in items.iter().filter(|item| !item.is_empty()) {
        rendered.push_str(&format!("  - {item}\n"));
    }
    match ticket.entry(field.key()) {
        // The whole entry, continuation lines included, is what a list replacement replaces.
        Some(entry) => edits.replace(entry.range(), rendered),
        None => edits.replace(insert_position(ticket, field), rendered),
    }
    Ok(())
}

/// Where a key that does not exist yet goes: before the first later field that does, so the result
/// reads in contract order rather than in the order things happened to be set.
fn insert_position(ticket: &Ticket, field: Field) -> std::ops::Range<usize> {
    let position = FIELDS
        .into_iter()
        .skip_while(|candidate| *candidate != field)
        .skip(1)
        .find_map(|later| ticket.entry(later.key()))
        .map(|entry| entry.start())
        .unwrap_or_else(|| ticket.frontmatter_insert_position());
    position..position
}

fn validate_status(previous: Option<&str>, next: &str) -> Result<()> {
    let next_status = Status::parse(next).ok_or_else(|| SkaldError::UnknownStatus {
        value: next.to_string(),
        permitted: STATUSES
            .iter()
            .map(|status| status.name().to_string())
            .collect(),
    })?;
    // Terminality is the only transition rule. Movement among the live states is free in any
    // direction, because whether a review finding is a small fix, a large hole, or a bad AC is
    // judgment, and a tool that guessed would be worked around.
    if let Some(current) = previous.and_then(Status::parse)
        && !current.may_move_to(next_status)
    {
        return Err(SkaldError::TerminalStatus {
            current: current.name(),
            next: next_status.name(),
        });
    }
    Ok(())
}

/// Refuse a mutation that would **introduce** a violation.
///
/// The rule is that skald must not create a violation it would later report — not that skald refuses
/// to touch an imperfect ticket. Those are different, and the difference is the whole migration: a
/// legacy ticket still carrying `phase:` has violations `--fix` deliberately won't repair, because
/// renaming a key is meaning rather than shape. Validating the whole ticket would make every such
/// ticket permanently unwritable — no log, no status change, nothing — and after the deny lands there
/// would be no way to rescue it.
///
/// So the comparison is against what was already wrong. Anything new is refused; anything already
/// there is somebody else's problem and does not block recording progress.
fn guard(store: &Store, before: Option<&Ticket>, after: &Ticket) -> Result<()> {
    let existing: Vec<String> = before
        .map(|ticket| {
            check::inspect(store, ticket)
                .iter()
                .map(|violation| violation.render_kind())
                .collect()
        })
        .unwrap_or_default();

    let introduced: Vec<String> = check::inspect(store, after)
        .iter()
        .filter(|violation| !existing.contains(&violation.render_kind()))
        .map(|violation| violation.render())
        .collect();

    match introduced.is_empty() {
        true => Ok(()),
        false => Err(SkaldError::WouldViolate(introduced)),
    }
}

/// Write via a sibling temp file and rename, so a crash or a full disk cannot leave a truncated
/// ticket. Truncating a ticket in place would take its whole log with it.
fn commit(ticket: &Ticket) -> Result<()> {
    let temp = ticket.path.with_extension("md.skald-tmp");
    let fail = |source: std::io::Error| SkaldError::WriteTicket(ticket.path.clone(), source);
    std::fs::write(&temp, ticket.source()).map_err(fail)?;
    // Same directory, so the rename is atomic and replaces the original in one step.
    std::fs::rename(&temp, &ticket.path).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::{Changes, log, new, set, set_criteria, today};
    use crate::check;
    use crate::errors::SkaldError;
    use crate::store::{Store, fixture};

    fn store() -> (std::path::PathBuf, Store) {
        let root = fixture("write", &[]);
        let store = Store::at(root.clone()).unwrap();
        (root, store)
    }

    fn scaffold(store: &Store, id: &str) -> crate::ticket::Ticket {
        new(
            store,
            id,
            &Changes {
                title: Some("A ticket".into()),
                repos: Some(vec!["skald".into()]),
                ..Changes::default()
            },
        )
        .unwrap();
        store.ticket(id).unwrap()
    }

    #[test]
    fn a_new_ticket_satisfies_check_immediately() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        assert!(
            check::inspect(&store, &ticket).is_empty(),
            "{:?}",
            check::inspect(&store, &ticket)
                .iter()
                .map(|v| v.kind.to_string())
                .collect::<Vec<_>>()
        );
        // Defaults: refining, and both owned sections present.
        assert_eq!(ticket.scalar("status"), Some("refining"));
        assert_eq!(ticket.scalar("created"), Some(today().as_str()));
        assert!(ticket.section("ac").is_some());
        assert!(ticket.section("log").is_some());
        // No H1, and no template file to drift from the contract.
        assert!(!ticket.source().contains("\n# "));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_refuses_to_overwrite() {
        let (root, store) = store();
        scaffold(&store, "t");
        let again = new(&store, "t", &Changes::default());
        assert!(matches!(again, Err(SkaldError::TicketExists(_))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn set_lands_several_fields_in_one_write_with_one_log_line() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set(
            &store,
            &ticket,
            &Changes {
                status: Some("building".into()),
                pr: Some("https://example.test/pr/1".into()),
                ..Changes::default()
            },
        )
        .unwrap();

        let after = store.ticket("t").unwrap();
        assert_eq!(after.scalar("status"), Some("building"));
        assert_eq!(after.scalar("pr"), Some("https://example.test/pr/1"));
        let log = after.section_body(after.section("log").unwrap());
        // One line, for the status move only — not for the pr.
        assert_eq!(log.matches("- ").count(), 1, "{log}");
        assert!(log.contains("status refining → building"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_status_that_does_not_move_leaves_no_log_line() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set(
            &store,
            &ticket,
            &Changes {
                status: Some("refining".into()),
                ..Changes::default()
            },
        )
        .unwrap();
        let after = store.ticket("t").unwrap();
        assert!(
            !after
                .section_body(after.section("log").unwrap())
                .contains("→")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_decorated_status_is_refused_and_names_the_bare_values() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        let refused = set(
            &store,
            &ticket,
            &Changes {
                status: Some("building (pending Ian)".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::UnknownStatus { .. })));
        // The file is untouched, so a refusal never half-applies.
        assert_eq!(
            store.ticket("t").unwrap().scalar("status"),
            Some("refining")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn title_colons_are_refused_for_new_and_set() {
        let (root, store) = store();
        let refused = new(
            &store,
            "bad-title",
            &Changes {
                title: Some("design: handoff".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::TitleContainsColon)));

        let ticket = scaffold(&store, "t");
        let refused = set(
            &store,
            &ticket,
            &Changes {
                title: Some("design: handoff".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::TitleContainsColon)));
        assert_eq!(store.ticket("t").unwrap().scalar("title"), Some("A ticket"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_terminal_status_is_terminal() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set(
            &store,
            &ticket,
            &Changes {
                status: Some("done".into()),
                ..Changes::default()
            },
        )
        .unwrap();

        let done = store.ticket("t").unwrap();
        for next in ["refining", "building", "reviewing", "cancelled"] {
            let refused = set(
                &store,
                &done,
                &Changes {
                    status: Some(next.into()),
                    ..Changes::default()
                },
            );
            assert!(
                matches!(refused, Err(SkaldError::TerminalStatus { .. })),
                "{next}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_criteria_are_frozen_outside_refining_with_no_way_around_it() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set_criteria(&store, &ticket, "- the original criterion").unwrap();

        let ticket = store.ticket("t").unwrap();
        set(
            &store,
            &ticket,
            &Changes {
                status: Some("building".into()),
                ..Changes::default()
            },
        )
        .unwrap();

        let building = store.ticket("t").unwrap();
        let refused = set_criteria(&store, &building, "- bent to fit the code");
        assert!(matches!(refused, Err(SkaldError::CriteriaFrozen { .. })));
        assert!(
            building
                .section_body(building.section("ac").unwrap())
                .contains("the original criterion")
        );

        // The way through is to say what you are doing, which leaves a trace.
        set(
            &store,
            &building,
            &Changes {
                status: Some("refining".into()),
                ..Changes::default()
            },
        )
        .unwrap();
        let refining = store.ticket("t").unwrap();
        set_criteria(&store, &refining, "- a deliberate revision").unwrap();

        let after = store.ticket("t").unwrap();
        assert!(
            after
                .section_body(after.section("ac").unwrap())
                .contains("a deliberate revision")
        );
        let log = after.section_body(after.section("log").unwrap());
        assert!(log.contains("status refining → building"));
        assert!(log.contains("status building → refining"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_written_ticket_stays_newline_terminated_and_keeps_its_sections_apart() {
        // Both found by walking the workflow rather than by a unit test: appending at the end of the
        // file left it unterminated, and replacing the criteria ate the blank line before `## Log`.
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set_criteria(&store, &ticket, "- a criterion").unwrap();
        let ticket = store.ticket("t").unwrap();
        log(&store, &ticket, "first").unwrap();
        let ticket = store.ticket("t").unwrap();
        log(&store, &ticket, "second").unwrap();

        let source = store.ticket("t").unwrap().source().to_string();
        assert!(source.ends_with('\n'), "{source:?}");
        assert!(!source.ends_with("\n\n"), "{source:?}");
        assert!(source.contains("- a criterion\n\n## Log"), "{source}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn criteria_written_when_the_section_is_last_do_not_gain_a_stray_blank_line() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        // Reverse order so Acceptance criteria is the final section.
        log(&store, &ticket, "an entry").unwrap();
        let ticket = store.ticket("t").unwrap();
        set_criteria(&store, &ticket, "- a criterion").unwrap();

        let source = store.ticket("t").unwrap().source().to_string();
        assert!(source.ends_with('\n'));
        assert!(!source.ends_with("\n\n"), "{source:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn log_dates_each_entry_and_only_ever_appends() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        log(&store, &ticket, "first").unwrap();
        let ticket = store.ticket("t").unwrap();
        log(&store, &ticket, "second").unwrap();

        let after = store.ticket("t").unwrap();
        let body = after.section_body(after.section("log").unwrap());
        let today = today();
        assert!(body.contains(&format!("- {today}: first")));
        assert!(body.contains(&format!("- {today}: second")));
        // Order preserved, and nothing lost.
        assert!(body.find("first").unwrap() < body.find("second").unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_mutation_bumps_updated_and_preserves_the_rest_byte_for_byte() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        let before = ticket.source().to_string();

        log(&store, &ticket, "an entry").unwrap();
        let after = store.ticket("t").unwrap();
        assert_eq!(after.scalar("updated"), Some(today().as_str()));

        // Every byte of the original survives as a prefix, and the only addition is the log entry
        // plus the blank line that separates it from its heading.
        let today = today();
        assert!(after.source().starts_with(&before), "{:?}", after.source());
        assert_eq!(
            &after.source()[before.len()..],
            format!("\n- {today}: an entry\n")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_ticket_that_is_already_wrong_can_still_record_progress() {
        // The rule is "do not create a violation", not "refuse to touch an imperfect ticket". A
        // legacy ticket carrying `phase:` has violations --fix deliberately will not repair, and
        // validating the whole ticket would make it permanently unwritable — with no rescue once the
        // deny lands.
        let legacy = "---\nphase: build\ntitle: t\nstatus: building\nrepos:\ncreated: 2026-08-20\nupdated: 2026-08-20\n---\n\n## Acceptance criteria\n\n## Log\n";
        let root = fixture("legacy-write", &[("t.md", legacy)]);
        let store = Store::at(root.clone()).unwrap();

        let ticket = store.ticket("t").unwrap();
        log(&store, &ticket, "still able to record what happened").unwrap();

        let ticket = store.ticket("t").unwrap();
        set(
            &store,
            &ticket,
            &Changes {
                status: Some("reviewing".into()),
                ..Changes::default()
            },
        )
        .unwrap();

        let after = store.ticket("t").unwrap();
        assert_eq!(after.scalar("status"), Some("reviewing"));
        assert!(
            after
                .section_body(after.section("log").unwrap())
                .contains("still able to record")
        );
        // The pre-existing violation is untouched and still reported.
        assert!(
            check::inspect(&store, &after)
                .iter()
                .any(|violation| violation.render_kind().contains("unknown key `phase`"))
        );

        // But a *new* violation is still refused.
        let refused = set(
            &store,
            &after,
            &Changes {
                pr: Some("not-a-url".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::WouldViolate(_))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_newline_in_a_value_is_refused_because_it_would_break_the_frontmatter() {
        // A bare line inside the block is neither a key nor a continuation: the parser ignores it,
        // `check` sees nothing wrong, and the frontmatter quietly stops being YAML.
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        let refused = set(
            &store,
            &ticket,
            &Changes {
                branch: Some("x\nbare line".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::NewlineInValue(_))));
        assert_eq!(store.ticket("t").unwrap().source(), ticket.source());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_list_shaped_value_is_refused_at_write_time() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        for value in ["[building (pending Ian)]", "[reviewing]"] {
            let refused = set(
                &store,
                &ticket,
                &Changes {
                    status: Some(value.into()),
                    ..Changes::default()
                },
            );
            assert!(
                matches!(refused, Err(SkaldError::UnknownStatus { .. })),
                "{value}"
            );
        }
        // And `new`, which has no prior version to compare against.
        let refused = new(
            &store,
            "u",
            &Changes {
                title: Some("U".into()),
                pr: Some("[https://x.dev/1, nope]".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::WouldViolate(_))));
        assert!(!store.contains("u"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_text_is_refused_rather_than_erasing_what_is_there() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set_criteria(&store, &ticket, "- the original criterion").unwrap();

        let ticket = store.ticket("t").unwrap();
        // The freeze exists to protect this content, so erasing it must not be what slips through.
        for text in ["", "   ", "\n\n"] {
            assert!(matches!(
                set_criteria(&store, &ticket, text),
                Err(SkaldError::EmptyText(_))
            ));
            assert!(matches!(
                log(&store, &ticket, text),
                Err(SkaldError::EmptyText(_))
            ));
        }
        assert!(
            store
                .ticket("t")
                .unwrap()
                .section_body(ticket.section("ac").unwrap())
                .contains("the original criterion")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_crlf_ticket_keeps_its_line_endings_through_a_write() {
        let crlf = "---\r\ntitle: t\r\nstatus: building\r\nrepos:\r\ncreated: 2026-08-20\r\nupdated: 2026-08-20\r\n---\r\n\r\n## Acceptance criteria\r\n\r\n## Log\r\n";
        let root = fixture("crlf", &[("t.md", crlf)]);
        let store = Store::at(root.clone()).unwrap();
        let ticket = store.ticket("t").unwrap();
        log(&store, &ticket, "an entry").unwrap();

        let after = store.ticket("t").unwrap();
        let source = after.source();
        // A file that used CRLF should not start accumulating bare newlines one append at a time.
        assert!(source.contains("an entry"));
        assert!(
            !source.replace("\r\n", "").contains('\n'),
            "mixed endings: {source:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn criteria_added_to_a_migrated_ticket_land_above_the_log() {
        let legacy = "---\ntitle: t\nstatus: refining\nrepos:\ncreated: 2026-08-20\nupdated: 2026-08-20\n---\n\n## Log\n\n- 2026-08-20: an entry\n";
        let root = fixture("insert-ac", &[("t.md", legacy)]);
        let store = Store::at(root.clone()).unwrap();
        let ticket = store.ticket("t").unwrap();
        set_criteria(&store, &ticket, "- a criterion").unwrap();

        let after = store.ticket("t").unwrap();
        let source = after.source();
        // The contract before the commentary; appending would read backwards.
        assert!(
            source.find("## Acceptance criteria").unwrap() < source.find("## Log").unwrap(),
            "{source}"
        );
        // The existing log entry survives the insertion above it.
        assert!(source.contains("2026-08-20: an entry"), "{source}");
        assert!(check::inspect(&store, &after).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_mutation_that_would_create_a_violation_fails_instead() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        // skald must not be able to write a value it would later report.
        let refused = set(
            &store,
            &ticket,
            &Changes {
                pr: Some("not-a-url".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(refused, Err(SkaldError::WouldViolate(_))));
        assert!(store.ticket("t").unwrap().entry("pr").is_some());
        assert_eq!(store.ticket("t").unwrap().scalar("pr"), None);

        let dangling = set(
            &store,
            &ticket,
            &Changes {
                parent: Some("nope".into()),
                ..Changes::default()
            },
        );
        assert!(matches!(dangling, Err(SkaldError::WouldViolate(_))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_a_field_is_the_same_gesture_for_every_field() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set(
            &store,
            &ticket,
            &Changes {
                paused: Some("waiting on Ian".into()),
                ..Changes::default()
            },
        )
        .unwrap();
        let paused = store.ticket("t").unwrap();
        assert_eq!(paused.scalar("paused"), Some("waiting on Ian"));

        set(
            &store,
            &paused,
            &Changes {
                paused: Some(String::new()),
                ..Changes::default()
            },
        )
        .unwrap();
        let resumed = store.ticket("t").unwrap();
        assert_eq!(resumed.scalar("paused"), None);
        // Resuming keeps the phase, which is why pause is a field rather than a status.
        assert_eq!(resumed.scalar("status"), Some("refining"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_list_field_is_replaced_wholesale_and_can_be_emptied() {
        let (root, store) = store();
        let ticket = scaffold(&store, "t");
        set(
            &store,
            &ticket,
            &Changes {
                repos: Some(vec!["skald".into(), "dotfiles".into()]),
                ..Changes::default()
            },
        )
        .unwrap();
        let two = store.ticket("t").unwrap();
        assert_eq!(two.list("repos"), vec!["skald", "dotfiles"]);

        set(
            &store,
            &two,
            &Changes {
                repos: Some(vec![String::new()]),
                ..Changes::default()
            },
        )
        .unwrap();
        let none = store.ticket("t").unwrap();
        assert!(none.list("repos").is_empty());
        assert!(check::inspect(&store, &none).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
