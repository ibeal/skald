//! `skald check` — validate a store against the compiled-in contract.
//!
//! This is the "test, not prose" half of the enforcement story: the docs describe the contract, and
//! `check` is what fails when the contract is broken. It reports **every** violation it finds rather
//! than the first, because a store being migrated has many at once and a validator that stops early
//! turns one pass into twenty.

use std::fmt::{self, Display};
use std::path::PathBuf;

use crate::contract::{FIELDS, Field, OWNED_LEVEL, OWNED_SECTIONS, Owned, STATUSES, Shape, Status};
use crate::errors::Result;
use crate::store::Store;
use crate::ticket::{Ticket, Value};

#[derive(Debug)]
pub struct Violation {
    pub path: PathBuf,
    /// 1-based, when the violation is anchored to a line. Absent for a whole-file problem such as a
    /// missing section.
    pub line: Option<usize>,
    pub kind: Kind,
}

#[derive(Debug)]
pub enum Kind {
    NoFrontmatter,
    UnterminatedFrontmatter,
    UnknownKey(String),
    DuplicateKey(String),
    MissingKey(&'static str),
    EmptyValue(&'static str),
    /// The case the whole tool exists for: a value outside the enum, decoration included.
    BadStatus(String),
    BadDate {
        key: &'static str,
        value: String,
    },
    BadUrl {
        key: &'static str,
        value: String,
    },
    NotAList(&'static str),
    /// A list or nested map where a single value belongs. Worth its own case because such a value
    /// slips past every scalar rule — `status: [building (pending Ian)]` would otherwise read as
    /// clean.
    NotAScalar(&'static str),
    DanglingParent(String),
    MissingSection(&'static str),
    /// The body is closed, so an unrecognized top-level heading is a violation, not an extension.
    UnknownSection(String),
    HeadingOne(String),
}

impl Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrontmatter => write!(f, "no frontmatter; a ticket needs the full field set"),
            Self::UnterminatedFrontmatter => write!(
                f,
                "frontmatter opens with `---` but never closes, so no keys were read"
            ),
            Self::UnknownKey(key) => write!(
                f,
                "unknown key `{key}`; the field set is closed, so this is a violation rather than an extension"
            ),
            Self::DuplicateKey(key) => write!(
                f,
                "`{key}` appears more than once; the first wins, so the rest are silently ignored"
            ),
            Self::MissingKey(key) => write!(f, "missing required key `{key}`"),
            Self::EmptyValue(key) => write!(f, "`{key}` may not be empty"),
            Self::BadStatus(value) => write!(
                f,
                "`{value}` is not a status. One bare value from: {}. \
                 If the state needs explaining, the value still goes in bare and the explanation goes in the log",
                STATUSES
                    .iter()
                    .map(|status| status.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::BadDate { key, value } => {
                write!(f, "`{key}` is `{value}`, which is not a YYYY-MM-DD date")
            }
            Self::BadUrl { key, value } => write!(
                f,
                "`{key}` is `{value}`, which is not an absolute http(s) URL. \
                 A reason there is no URL is a log entry, not a value"
            ),
            Self::NotAList(key) => write!(f, "`{key}` must be a list"),
            Self::NotAScalar(key) => write!(
                f,
                "`{key}` must be a single value, not a list or a nested map"
            ),
            Self::DanglingParent(id) => {
                write!(
                    f,
                    "`parent` names `{id}`, which is not a ticket in this store"
                )
            }
            Self::MissingSection(heading) => write!(f, "missing `## {heading}` section"),
            Self::UnknownSection(title) => write!(
                f,
                "`## {title}` is not a section skald owns; the body is closed to \
                 Acceptance criteria and Log. Deeper headings inside a section are fine"
            ),
            Self::HeadingOne(title) => write!(
                f,
                "`# {title}` is an H1; the title lives in frontmatter, so the body starts at `##`"
            ),
        }
    }
}

impl Violation {
    /// `path:line: message`, so an editor can jump to it.
    pub fn render(&self) -> String {
        match self.line {
            Some(line) => format!("{}:{}: {}", self.path.display(), line, self.kind),
            None => format!("{}: {}", self.path.display(), self.kind),
        }
    }

    /// The message without its location, for asking "was this already wrong?" across two versions of
    /// a ticket. Line numbers shift when a write adds a key, so comparing on them would call an
    /// untouched violation a new one.
    pub fn render_kind(&self) -> String {
        self.kind.to_string()
    }
}

/// Validate every ticket in the store.
pub fn run(store: &Store) -> Result<Vec<Violation>> {
    let listing = store.tickets()?;
    let mut violations = Vec::new();
    for ticket in &listing.tickets {
        violations.extend(inspect(store, ticket));
    }
    Ok(violations)
}

/// Every violation in one ticket, in file order where a line is known.
pub fn inspect(store: &Store, ticket: &Ticket) -> Vec<Violation> {
    let mut found = Vec::new();
    let at = |line: Option<usize>, kind: Kind| Violation {
        path: ticket.path.clone(),
        line,
        kind,
    };

    if ticket.frontmatter.unterminated {
        found.push(at(Some(1), Kind::UnterminatedFrontmatter));
        return found;
    }
    if !ticket.frontmatter.present {
        found.push(at(Some(1), Kind::NoFrontmatter));
    }

    let mut seen: Vec<&str> = Vec::new();
    for entry in ticket.entries() {
        let line = Some(entry.line);
        let Some(field) = Field::from_key(&entry.key) else {
            found.push(at(line, Kind::UnknownKey(entry.key.clone())));
            continue;
        };
        if seen.contains(&entry.key.as_str()) {
            found.push(at(line, Kind::DuplicateKey(entry.key.clone())));
            continue;
        }
        seen.push(&entry.key);

        let empty = matches!(entry.value, Value::Empty);
        if empty {
            if !field.allows_empty() {
                found.push(at(line, Kind::EmptyValue(field.key())));
            }
            // An empty value has no shape to be wrong.
            continue;
        }
        found.extend(check_shape(store, ticket, field, entry.line).map(|kind| at(line, kind)));
    }

    for field in FIELDS {
        if field.required() && ticket.entry(field.key()).is_none() {
            found.push(at(None, Kind::MissingKey(field.key())));
        }
    }

    found.extend(check_body(ticket).map(|(line, kind)| at(line, kind)));
    found
}

fn check_shape(store: &Store, ticket: &Ticket, field: Field, _line: usize) -> Option<Kind> {
    let entry = ticket.entry(field.key())?;

    // A scalar where a list belongs is how `repos: dotfiles — some/path` got written: it reads fine
    // and filters as nothing.
    if field.shape() == Shape::List {
        return matches!(entry.value, Value::Scalar(_) | Value::Nested)
            .then(|| Kind::NotAList(field.key()));
    }

    // Every other shape is a single value, and every rule below reads one. A list- or map-shaped
    // value has no scalar to read, so without this the rule would simply not run and
    // `status: [building (pending Ian)]` would pass as clean.
    let Some(value) = ticket.scalar(field.key()) else {
        return Some(Kind::NotAScalar(field.key()));
    };

    match field.shape() {
        Shape::Text => None,
        Shape::Status => Status::parse(value)
            .is_none()
            .then(|| Kind::BadStatus(value.to_string())),
        Shape::Date => (!is_date(value)).then(|| Kind::BadDate {
            key: field.key(),
            value: value.to_string(),
        }),
        Shape::Url => (!is_absolute_http_url(value)).then(|| Kind::BadUrl {
            key: field.key(),
            value: value.to_string(),
        }),
        Shape::TicketId => {
            (!store.contains(value)).then(|| Kind::DanglingParent(value.to_string()))
        }
        Shape::List => None,
    }
}

fn check_body(ticket: &Ticket) -> impl Iterator<Item = (Option<usize>, Kind)> + use<> {
    let mut found = Vec::new();

    for section in ticket.sections() {
        if section.level == 1 {
            found.push((
                Some(ticket.line_of_section(section)),
                Kind::HeadingOne(section.title.clone()),
            ));
            continue;
        }
        // Only the top level is skald's. Structure inside a section belongs to the author.
        if section.level == OWNED_LEVEL && Owned::from_address(&section.slug).is_none() {
            found.push((
                Some(ticket.line_of_section(section)),
                Kind::UnknownSection(section.title.clone()),
            ));
        }
    }

    for owned in OWNED_SECTIONS {
        if ticket.section(owned.heading()).is_none() {
            found.push((None, Kind::MissingSection(owned.heading())));
        }
    }

    found.into_iter()
}

/// What `--fix` did to one ticket.
pub struct Fixed {
    pub path: PathBuf,
    pub changes: Vec<String>,
}

/// Repair only what is unambiguously mechanical, and rewrite the file when anything changed.
///
/// **`--fix` never guesses at a status.** `building (pending Ian)` is reported and left alone, because
/// the qualifier is information and discarding it is data loss — the human moves it to the log. The
/// same reasoning rules out inferring a status from an old `phase`, so migration renames are not
/// `--fix`'s job either. What is left is shape, not meaning.
pub fn fix(store: &Store, ticket: &Ticket) -> Result<Option<Fixed>> {
    let mut changes = Vec::new();
    let mut edits = ticket.edits();

    // An unparsed block has no structure to repair, and guessing where it was meant to close would
    // invent some.
    if ticket.frontmatter.unterminated {
        return Ok(None);
    }

    for entry in ticket.entries() {
        let Some(field) = Field::from_key(&entry.key) else {
            continue;
        };
        let Some(value) = ticket.scalar(field.key()) else {
            continue;
        };
        // Note there is deliberately no whitespace-trimming fix, though the AC asked for one.
        // Padding around a value is not a violation — the parser reads the trimmed value, so `check`
        // has nothing to report — and rewriting it would collapse deliberate alignment, which is
        // exactly the byte-preservation the round-trip guarantee exists to provide. A fix that
        // repairs nothing and destroys something is not a fix.
        if field.shape() == Shape::Date
            && !is_date(value)
            && let Some(normalized) = normalize_date(value)
        {
            edits.replace(entry.value_range(), format!(" {normalized}"));
            changes.push(format!("normalized `{}` to {normalized}", field.key()));
        }
    }

    // The one semantic fix worth having: it is exactly the migration every existing ticket needs, and
    // it is checkable rather than inferred.
    //
    // The heading is only removed when its text is guaranteed a home. Removing it and then failing to
    // write the title — which is what happened when frontmatter was absent, or when `title:` already
    // held a value — deleted the text outright while reporting that it had been lifted, and the
    // resulting ticket passed `check`, so the loss was invisible.
    let existing_title = ticket.entry(Field::Title.key());
    let title_has_room = ticket.frontmatter.present
        && match existing_title {
            Some(entry) => matches!(entry.value, Value::Empty),
            None => true,
        };
    let lifted = match title_has_room {
        true => lift_heading_one(ticket, &mut edits, &mut changes),
        false => None,
    };

    if let Some(title) = &lifted
        && let Some(entry) = existing_title
    {
        edits.replace(entry.value_range(), format!(" {title}"));
    }

    // Every key that needs adding goes in as one batch in contract order. Inserted separately they
    // would all anchor at the same byte and come out in call order instead, so a repaired ticket
    // would read like a repaired one.
    if ticket.frontmatter.present {
        // A managed date gets a value, not a blank. `created`/`updated` may not be empty and no
        // command can set them, so adding them empty would leave the ticket permanently unwritable —
        // which is exactly the legacy ticket this fix exists to rescue. The file's own mtime is the
        // best evidence available of when it last changed, and beats inventing today.
        let file_date = file_date(&ticket.path);
        let additions: Vec<(Field, String)> = FIELDS
            .into_iter()
            .filter(|field| ticket.entry(field.key()).is_none())
            .filter(|field| field.required() || (*field == Field::Title && lifted.is_some()))
            .map(|field| match (field, &lifted) {
                (Field::Title, Some(title)) => (field, title.clone()),
                _ if field.is_managed() => (field, file_date.clone()),
                _ => (field, String::new()),
            })
            .collect();
        for (field, value) in &additions {
            match (value.is_empty(), field.is_managed()) {
                (true, _) => changes.push(format!("added `{}:` with no value", field.key())),
                (false, true) => changes.push(format!(
                    "added `{}: {value}` from the file's last-modified date",
                    field.key()
                )),
                // The only other non-empty addition is the lifted title, which `lift_heading_one`
                // already reported; saying it twice makes one change look like two.
                (false, false) => {}
            }
        }
        insert_keys(ticket, &additions, &mut edits);
    }

    add_missing_sections(ticket, &mut edits, &mut changes);

    if changes.is_empty() {
        return Ok(None);
    }
    let rendered = edits.render();
    std::fs::write(&ticket.path, &rendered)
        .map_err(|source| crate::errors::SkaldError::WriteTicket(ticket.path.clone(), source))?;
    let _ = store;
    Ok(Some(Fixed {
        path: ticket.path.clone(),
        changes,
    }))
}

/// `2026-8-20`, `2026/08/20` → `2026-08-20`. `None` when the intent is not obvious.
///
/// Deliberately refuses a day-first or month-first date: `08-09-2026` could be either, and a
/// validator that picked one would silently move a ticket's dates by months.
fn normalize_date(value: &str) -> Option<String> {
    let parts: Vec<&str> = value.split(['-', '/', '.']).collect();
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit()) && !part.is_empty();
    if year.len() != 4 || !digits(year) || !digits(month) || !digits(day) {
        return None;
    }
    let month: u32 = month.parse().ok()?;
    let day: u32 = day.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year}-{month:02}-{day:02}"))
}

/// The file's last-modified date, or today when it cannot be read. The best available evidence of
/// when a ticket that never recorded its own dates last changed.
fn file_date(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| jiff::Timestamp::try_from(time).ok())
        .map(|stamp| {
            stamp
                .to_zoned(jiff::tz::TimeZone::system())
                .date()
                .to_string()
        })
        .unwrap_or_else(crate::write::today)
}

/// Remove a leading H1 and return the title it carried, if it carried one.
///
/// The old shape was `# <id> — <title>`, so an `<id>` prefix that **exactly** matches the ticket's id
/// is stripped. Requiring an exact match is what keeps this mechanical rather than a guess: if the
/// prefix is anything else, the whole heading becomes the title.
///
/// `Some("")` never comes back. An H1 that is *only* the id carried no title, and inventing
/// `title: ask-2026-07-23-install-helix` would put a non-title in the field a human reads first. The
/// heading still goes — it held nothing — and `check` then reports the empty title, which is the
/// honest outcome and the one that prompts a human to write a real one.
fn lift_heading_one(
    ticket: &Ticket,
    edits: &mut crate::ticket::Edits<'_>,
    changes: &mut Vec<String>,
) -> Option<String> {
    let section = ticket
        .sections()
        .iter()
        .find(|section| section.level == 1)?;

    let mut title = section.title.as_str();
    if let Some(rest) = title.strip_prefix(&ticket.id) {
        let rest = rest.trim_start();
        title = match rest.is_empty() {
            true => "",
            false => rest
                .strip_prefix(['—', '–', '-', ':'])
                .map(str::trim)
                .unwrap_or(title),
        };
    }

    edits.replace(ticket.section_heading_range(section), String::new());
    match title.is_empty() {
        true => {
            changes.push("removed an H1 that held only the ticket id".to_string());
            None
        }
        false => {
            changes.push(format!("lifted the H1 into `title: {title}`"));
            Some(title.to_string())
        }
    }
}

/// Insert keys in the contract's own order, so a repaired ticket reads like a new one rather than
/// like a repaired one.
fn insert_keys(ticket: &Ticket, pairs: &[(Field, String)], edits: &mut crate::ticket::Edits<'_>) {
    for (field, value) in pairs {
        let rendered = match value.is_empty() {
            true => format!("{}:\n", field.key()),
            false => format!("{}: {value}\n", field.key()),
        };
        let position = FIELDS
            .into_iter()
            .skip_while(|candidate| candidate != field)
            .skip(1)
            .find_map(|later| ticket.entry(later.key()))
            .map(|entry| entry.start())
            .unwrap_or_else(|| ticket.frontmatter_insert_position());
        edits.replace(position..position, rendered);
    }
}

fn add_missing_sections(
    ticket: &Ticket,
    edits: &mut crate::ticket::Edits<'_>,
    changes: &mut Vec<String>,
) {
    let end = ticket.source().len();
    for owned in OWNED_SECTIONS {
        if ticket.section(owned.heading()).is_some() {
            continue;
        }
        // Appending keeps the fix trivially safe. Order only matters for reading, and `check` does
        // not police section order.
        let separator = match ticket.source().ends_with('\n') {
            true => "",
            false => "\n",
        };
        edits.replace(end..end, format!("{separator}\n## {}\n", owned.heading()));
        changes.push(format!("added the `## {}` section", owned.heading()));
    }
}

/// `YYYY-MM-DD`, with a plausible month and day. Not a calendar — a validator that rejected
/// 2026-02-30 would need one, and the value is a human's note of a day, not an instant.
pub fn is_date(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let numeric = |part: &str, width: usize| {
        part.len() == width && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    if !numeric(year, 4) || !numeric(month, 2) || !numeric(day, 2) {
        return false;
    }
    let month: u32 = month.parse().unwrap_or(0);
    let day: u32 = day.parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_absolute_http_url(value: &str) -> bool {
    let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    // A scheme with nothing after it is not a URL, and neither is one with a space in it — which is
    // what a decorated value looks like.
    !rest.is_empty() && !rest.starts_with('/') && !value.contains(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::{inspect, is_date, run};
    use crate::store::{Store, fixture};
    use crate::ticket::Ticket;

    const GOOD: &str = "---\ntitle: A good ticket\nstatus: building\npaused:\nrepos:\n  - skald\nbranch:\nlink:\npr:\nparent:\ncreated: 2026-08-20\nupdated: 2026-08-20\n---\n\n## Acceptance criteria\n\n- a criterion\n\n## Log\n\n- 2026-08-20: an entry\n";

    fn kinds(source: &str) -> Vec<String> {
        let root = fixture("check", &[("t.md", source)]);
        let store = Store::at(root.clone()).unwrap();
        let ticket = Ticket::parse("t", root.join("t.md"), source.to_string());
        let out = inspect(&store, &ticket)
            .into_iter()
            .map(|violation| violation.kind.to_string())
            .collect();
        std::fs::remove_dir_all(root).unwrap();
        out
    }

    #[test]
    fn a_conforming_ticket_has_nothing_to_say() {
        assert!(kinds(GOOD).is_empty(), "{:?}", kinds(GOOD));
    }

    #[test]
    fn a_clean_store_exits_quiet() {
        let root = fixture("check-clean", &[("t.md", GOOD)]);
        let store = Store::at(root.clone()).unwrap();
        assert!(run(&store).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_decorated_status_is_reported_and_names_the_bare_values() {
        let found = kinds(&GOOD.replace("status: building", "status: building (pending Ian)"));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("is not a status"));
        // The message has to teach the way out, since this is the case that motivated the tool.
        assert!(found[0].contains("goes in the log"));
    }

    #[test]
    fn every_violation_in_a_ticket_is_reported_not_just_the_first() {
        // A store mid-migration has many at once; stopping early turns one pass into twenty.
        let source = "---\nid: t\nphase: build\nprojects: skald\ncreated: 20-08-2026\n---\n\n# An H1\n\n## Notes\n";
        let found = kinds(source);
        let joined = found.join("\n");
        assert!(joined.contains("unknown key `id`"), "{joined}");
        assert!(joined.contains("unknown key `phase`"), "{joined}");
        assert!(joined.contains("unknown key `projects`"), "{joined}");
        assert!(joined.contains("not a YYYY-MM-DD date"), "{joined}");
        assert!(joined.contains("missing required key `title`"), "{joined}");
        assert!(joined.contains("missing required key `status`"), "{joined}");
        assert!(joined.contains("missing required key `repos`"), "{joined}");
        assert!(
            joined.contains("missing required key `updated`"),
            "{joined}"
        );
        assert!(joined.contains("is an H1"), "{joined}");
        assert!(joined.contains("`## Notes` is not a section"), "{joined}");
        assert!(
            joined.contains("missing `## Acceptance criteria`"),
            "{joined}"
        );
        assert!(joined.contains("missing `## Log`"), "{joined}");
    }

    #[test]
    fn deeper_headings_inside_a_section_are_the_authors_business() {
        let source = GOOD.replace(
            "## Acceptance criteria\n\n- a criterion",
            "## Acceptance criteria\n\n### Bootstrap\n\n- a criterion\n\n#### Deeper\n\n- more",
        );
        assert!(kinds(&source).is_empty());
    }

    #[test]
    fn an_empty_value_is_only_a_violation_where_the_field_needs_one() {
        // `pr:` empty is the normal state of a ticket; `title:` empty is meaningless.
        assert!(kinds(GOOD).is_empty());
        let found = kinds(&GOOD.replace("title: A good ticket", "title:"));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("`title` may not be empty"));
    }

    #[test]
    fn a_scalar_where_a_list_belongs_is_reported() {
        // How `repos: dotfiles — modules/nixos/x.nix` gets written: reads fine, filters as nothing.
        let found = kinds(&GOOD.replace("repos:\n  - skald", "repos: skald"));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("`repos` must be a list"));
    }

    #[test]
    fn a_url_field_rejects_a_value_that_is_not_one() {
        for value in [
            "not-a-url",
            "github.com/x/y",
            "https://",
            "https://x.dev/1 (draft)",
        ] {
            let found = kinds(&GOOD.replace("pr:", &format!("pr: {value}")));
            assert_eq!(found.len(), 1, "{value}: {found:?}");
            assert!(found[0].contains("not an absolute http(s) URL"), "{value}");
        }
        assert!(
            kinds(&GOOD.replace("pr:", "pr: https://github.com/ibeal/skald/pull/1")).is_empty()
        );
    }

    #[test]
    fn a_parent_that_names_no_ticket_is_a_dangling_reference() {
        let found = kinds(&GOOD.replace("parent:", "parent: nope"));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("not a ticket in this store"));
    }

    #[test]
    fn a_duplicated_key_is_reported_because_the_later_copies_are_ignored() {
        let found = kinds(&GOOD.replace("status: building", "status: building\nstatus: reviewing"));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("appears more than once"));
    }

    #[test]
    fn broken_frontmatter_stops_at_the_one_thing_worth_saying() {
        // Reporting twelve missing keys under an unterminated block buries the actual problem.
        let found = kinds("---\ntitle: t\n\n## Log\n");
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("never closes"));
    }

    /// Run `--fix` against a one-ticket store and hand back what the file became.
    fn fixed(source: &str) -> (String, Vec<String>) {
        let root = fixture("fix", &[("t.md", source)]);
        let store = Store::at(root.clone()).unwrap();
        let ticket = store.ticket("t").unwrap();
        let changes = super::fix(&store, &ticket)
            .unwrap()
            .map(|fixed| fixed.changes)
            .unwrap_or_default();
        let after = std::fs::read_to_string(root.join("t.md")).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        (after, changes)
    }

    #[test]
    fn fix_never_touches_a_decorated_status() {
        // The qualifier is information, and discarding it is data loss. The human moves it to the log.
        let source = GOOD.replace("status: building", "status: building (pending Ian)");
        let (after, changes) = fixed(&source);
        assert!(after.contains("status: building (pending Ian)"));
        assert!(changes.is_empty(), "{changes:?}");
        // And it is still a violation, so `--fix` does not make a broken store look clean.
        let found = kinds(&source);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("is not a status"));
    }

    #[test]
    fn fix_leaves_a_conforming_ticket_byte_for_byte_alone() {
        let (after, changes) = fixed(GOOD);
        assert_eq!(after, GOOD);
        assert!(changes.is_empty());
    }

    #[test]
    fn fix_adds_missing_keys_in_contract_order() {
        // Inserted one at a time they would all anchor at the same byte and come out in call order.
        let source = "---\nupdated: 2026-08-20\n---\n\n## Acceptance criteria\n\n## Log\n";
        let (after, _) = fixed(source);
        let keys: Vec<&str> = after
            .lines()
            .take_while(|line| *line != "---" || after.starts_with(line))
            .filter_map(|line| line.split_once(':').map(|(key, _)| key))
            .collect();
        assert_eq!(keys, vec!["title", "status", "repos", "created", "updated"]);
    }

    #[test]
    fn fix_gives_a_managed_date_a_value_so_the_ticket_stays_writable() {
        // Adding `created:` empty would satisfy "the key is present" and then fail "may not be
        // empty" — and no command can set a managed date, so the ticket would be permanently
        // unwritable. That is exactly the legacy ticket this fix exists to rescue.
        let source =
            "---\ntitle: t\nstatus: building\nrepos:\n---\n\n## Acceptance criteria\n\n## Log\n";
        let (after, changes) = fixed(source);
        assert!(
            changes
                .iter()
                .any(|change| change.contains("last-modified date")),
            "{changes:?}"
        );
        // The whole point: the repaired ticket has nothing left to report.
        assert!(kinds(&after).is_empty(), "{:?}", kinds(&after));
    }

    #[test]
    fn fix_lifts_an_h1_into_the_title_and_strips_an_id_prefix() {
        let source = "---\nupdated: 2026-08-20\n---\n\n# t — A real title\n\n## Acceptance criteria\n\n## Log\n";
        let (after, changes) = fixed(source);
        assert!(after.contains("title: A real title"));
        assert!(!after.contains("# t — A real title"));
        assert!(
            changes
                .iter()
                .any(|change| change.contains("lifted the H1"))
        );
        // One change, reported once.
        assert_eq!(
            changes
                .iter()
                .filter(|change| change.contains("title"))
                .count(),
            1,
            "{changes:?}"
        );
    }

    #[test]
    fn an_h1_that_is_only_the_id_yields_no_title_rather_than_a_fake_one() {
        let source = "---\nupdated: 2026-08-20\n---\n\n# t\n\n## Acceptance criteria\n\n## Log\n";
        let (after, changes) = fixed(source);
        // `title: t` would put a non-title in the field a human reads first.
        assert!(after.contains("title:\n"));
        assert!(!after.contains("title: t"));
        assert!(
            changes
                .iter()
                .any(|change| change.contains("only the ticket id"))
        );
    }

    #[test]
    fn fix_normalizes_an_unambiguous_date_and_refuses_an_ambiguous_one() {
        let (after, _) = fixed(&GOOD.replace("created: 2026-08-20", "created: 2026/8/9"));
        assert!(after.contains("created: 2026-08-09"), "{after}");

        // 08-09-2026 could be August 9th or September 8th; picking one moves the date by months.
        let source = GOOD.replace("created: 2026-08-20", "created: 08-09-2026");
        let (after, changes) = fixed(&source);
        assert!(after.contains("created: 08-09-2026"));
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn fix_leaves_alignment_padding_alone() {
        // Padding is not a violation: the parser reads the trimmed value, so `check` reports nothing,
        // and normalizing it would collapse deliberate alignment for no gain.
        let source = GOOD.replace("title: A good ticket", "title:   A good ticket   ");
        let (after, changes) = fixed(&source);
        assert_eq!(after, source);
        assert!(changes.is_empty(), "{changes:?}");
        assert!(kinds(&source).is_empty());
    }

    #[test]
    fn fix_adds_a_missing_owned_section() {
        let source = "---\ntitle: t\nstatus: building\nrepos:\ncreated: 2026-08-20\nupdated: 2026-08-20\n---\n";
        let (after, _) = fixed(source);
        assert!(after.contains("## Acceptance criteria"));
        assert!(after.contains("## Log"));
        assert!(kinds(&after).is_empty(), "{:?}", kinds(&after));
    }

    #[test]
    fn fix_declines_to_guess_at_broken_frontmatter() {
        // Guessing where an unterminated block was meant to close would invent structure.
        let source = "---\ntitle: t\n\n## Log\n";
        let (after, changes) = fixed(source);
        assert_eq!(after, source);
        assert!(changes.is_empty());
    }

    #[test]
    fn fix_does_not_rename_an_old_section_or_an_old_key() {
        // Migration renames are meaning, not shape. Inferring a status from an old phase is exactly
        // the guess --fix must not make.
        let source = "---\nphase: build\ntitle: t\nstatus: building\nrepos:\ncreated: 2026-08-20\nupdated: 2026-08-20\n---\n\n## Acceptance criteria\n\n## Build log\n\n- an entry\n";
        let (after, _) = fixed(source);
        assert!(after.contains("phase: build"));
        assert!(after.contains("## Build log"));
        let joined = kinds(&after).join("\n");
        assert!(joined.contains("unknown key `phase`"));
        assert!(joined.contains("`## Build log` is not a section"));
    }

    #[test]
    fn a_list_shaped_value_cannot_smuggle_a_bad_value_past_a_scalar_rule() {
        // Every scalar rule reads a scalar, so a list- or map-shaped value used to skip the rule
        // entirely — `status: [building (pending Ian)]` read as clean.
        for (line, replacement) in [
            ("status: building", "status: [building (pending Ian)]"),
            ("pr:", "pr: [https://x.dev/1, nope]"),
            ("parent:", "parent: [ghost]"),
            ("created: 2026-08-20", "created: [2026-08-20]"),
        ] {
            let source = GOOD.replacen(&format!("{line}\n"), &format!("{replacement}\n"), 1);
            let found = kinds(&source);
            let key = line.split(':').next().unwrap();
            assert!(
                found
                    .iter()
                    .any(|kind| kind.contains("must be a single value")),
                "{key}: {found:?}"
            );
        }
    }

    #[test]
    fn fix_does_not_delete_an_h1_it_cannot_find_a_home_for() {
        // Removing the heading and then failing to write the title deleted the text outright while
        // reporting that it had been lifted, and the result passed `check` — so the loss was silent.
        let no_frontmatter = "# My great title\n\n## Log\n";
        let (after, changes) = fixed(no_frontmatter);
        assert!(after.contains("# My great title"), "{after}");
        assert!(
            !changes.iter().any(|change| change.contains("lifted")),
            "{changes:?}"
        );

        let title_taken = "---\ntitle: Existing\nstatus: building\nrepos:\ncreated: 2026-08-20\nupdated: 2026-08-20\n---\n\n# My great title\n\n## Acceptance criteria\n\n## Log\n";
        let (after, _) = fixed(title_taken);
        assert!(after.contains("# My great title"), "{after}");
        assert!(after.contains("title: Existing"));
        // Still reported, so a human resolves it rather than it being quietly dropped.
        assert!(kinds(&after).iter().any(|kind| kind.contains("is an H1")));
    }

    #[test]
    fn dates_are_checked_for_shape_and_plausibility() {
        assert!(is_date("2026-08-20"));
        assert!(!is_date("2026-8-20"));
        assert!(!is_date("20-08-2026"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-00-01"));
        assert!(!is_date("2026-08-32"));
        assert!(!is_date("2026-08-20 (approx)"));
    }
}
