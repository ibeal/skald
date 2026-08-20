//! `skald show` — the read a resuming agent starts from.

use serde_json::{Map, Value as Json, json};

use crate::errors::{Result, SkaldError};
use crate::ticket::{Section, Ticket, Value};

/// A whole ticket, or one section, exactly as it appears in the file.
///
/// With no `--section` this is the file's own bytes: a resuming agent orients in one call, and what
/// it reads is what is on disk rather than a re-rendering that might differ.
pub fn render_text(ticket: &Ticket, section: Option<&str>) -> Result<String> {
    Ok(match resolve(ticket, section)? {
        None => ticket.source().to_string(),
        Some(section) => ticket.section_text(section).to_string(),
    })
}

pub fn render_json(ticket: &Ticket, section: Option<&str>) -> Result<Json> {
    if let Some(section) = resolve(ticket, section)? {
        return Ok(json!({
            "id": ticket.id,
            "path": ticket.path.display().to_string(),
            "section": section_json(ticket, section),
        }));
    }

    Ok(json!({
        "id": ticket.id,
        "path": ticket.path.display().to_string(),
        "frontmatter": frontmatter_json(ticket),
        // An opening `---` with no closing one leaves the whole block unparsed. Saying so is the
        // difference between "this ticket has no frontmatter" and "this ticket's frontmatter is
        // broken", and a reader that can't tell those apart will write a second copy of every key.
        "frontmatter_unterminated": ticket.frontmatter.unterminated,
        "sections": ticket.sections().iter().map(|section| section_json(ticket, section)).collect::<Vec<_>>(),
        // The bytes travel with the structure, so one call serves both an agent that wants fields
        // and one that wants to read the ticket as written.
        "raw": ticket.source(),
    }))
}

/// A one-line warning for a ticket whose frontmatter could not be parsed, or `None` when it is fine.
///
/// Printed to stderr so it reaches a human without touching what `show` writes to stdout, which has
/// to stay byte-identical to the file.
pub fn frontmatter_warning(ticket: &Ticket) -> Option<String> {
    ticket.frontmatter.unterminated.then(|| {
        format!(
            "warning: {}: frontmatter opens with `---` but never closes, so no keys were read",
            ticket.path.display()
        )
    })
}

fn resolve<'a>(ticket: &'a Ticket, section: Option<&str>) -> Result<Option<&'a Section>> {
    let Some(name) = section else {
        return Ok(None);
    };
    ticket
        .section(name)
        .map(Some)
        .ok_or_else(|| SkaldError::UnknownSection {
            id: ticket.id.clone(),
            section: name.to_string(),
            available: ticket.section_addresses(),
        })
}

fn section_json(ticket: &Ticket, section: &Section) -> Json {
    json!({
        "title": section.title,
        "slug": section.slug,
        "level": section.level,
        "body": ticket.section_body(section),
    })
}

/// Frontmatter as JSON, in the file's own key order.
///
/// A duplicated key keeps its **first** occurrence, matching `Ticket::entry` — which is what `list`
/// filters on and what a `set` rewrites. Letting the last win here instead would make a successful
/// `set` look like a no-op to anything reading this output. `check` is what reports the duplicate.
pub fn frontmatter_json(ticket: &Ticket) -> Json {
    let mut map = Map::new();
    for entry in ticket.entries() {
        if map.contains_key(&entry.key) {
            continue;
        }
        let value = match &entry.value {
            Value::Empty => Json::Null,
            Value::Scalar(value) => Json::String(value.clone()),
            Value::List(items) => Json::from(items.clone()),
            // skald does not own the shape of a store's nested keys, so it hands back the bytes
            // rather than inventing a structure for them.
            Value::Nested => json!({ "unparsed": ticket.entry_text(entry) }),
        };
        map.insert(entry.key.clone(), value);
    }
    Json::Object(map)
}

#[cfg(test)]
mod tests {
    use super::{render_json, render_text};
    use crate::errors::SkaldError;
    use crate::ticket::Ticket;

    const SOURCE: &str = "\
---
id: one
phase: build
pr:
projects:
  - skald
---

# One

## Journal

### Build log

- 2026-08-20: first.
";

    fn ticket() -> Ticket {
        Ticket::parse("one", "/store/one.md", SOURCE.to_string())
    }

    #[test]
    fn showing_a_whole_ticket_reproduces_the_file_exactly() {
        assert_eq!(render_text(&ticket(), None).unwrap(), SOURCE);
    }

    #[test]
    fn showing_a_section_returns_its_heading_and_body_verbatim() {
        let text = render_text(&ticket(), Some("build-log")).unwrap();
        assert_eq!(text, "### Build log\n\n- 2026-08-20: first.\n");
    }

    #[test]
    fn an_unknown_section_lists_the_ones_the_ticket_has() {
        let error = render_text(&ticket(), Some("Review findings")).unwrap_err();
        assert!(matches!(
            &error,
            SkaldError::UnknownSection { available, .. }
                if available == &["One", "Journal", "Build log"]
        ));
    }

    #[test]
    fn a_duplicated_key_reads_the_same_here_as_everywhere_else() {
        let ticket = Ticket::parse(
            "dup",
            "/store/dup.md",
            "---\nphase: build\nphase: review\n---\n\n# Dup\n".to_string(),
        );
        // First wins, matching what `list` filters on and what a `set` would rewrite. Last-wins here
        // would make a successful `set` look like a no-op to anything reading this output.
        assert_eq!(ticket.scalar("phase"), Some("build"));
        let json = render_json(&ticket, None).unwrap();
        assert_eq!(json["frontmatter"]["phase"], "build");
    }

    #[test]
    fn broken_frontmatter_is_distinguishable_from_absent_frontmatter() {
        let broken = Ticket::parse(
            "broken",
            "/store/broken.md",
            "---\nid: broken\n\n# Title\n".to_string(),
        );
        let json = render_json(&broken, None).unwrap();
        assert_eq!(json["frontmatter_unterminated"], true);
        // Empty-and-broken must not read as empty-and-fine, or a caller writes a second copy of
        // every key.
        assert!(json["frontmatter"].as_object().unwrap().is_empty());
        assert!(super::frontmatter_warning(&broken).is_some());

        let absent = Ticket::parse("plain", "/store/plain.md", "# Title\n".to_string());
        assert_eq!(
            render_json(&absent, None).unwrap()["frontmatter_unterminated"],
            false
        );
        assert!(super::frontmatter_warning(&absent).is_none());
    }

    #[test]
    fn json_keeps_frontmatter_order_and_carries_the_raw_bytes() {
        let json = render_json(&ticket(), None).unwrap();
        let keys: Vec<&String> = json["frontmatter"].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["id", "phase", "pr", "projects"]);
        // An unfilled field is null, not the empty string: absent-of-value, not a value.
        assert!(json["frontmatter"]["pr"].is_null());
        assert_eq!(json["frontmatter"]["projects"][0], "skald");
        assert_eq!(json["raw"], SOURCE);
    }
}
