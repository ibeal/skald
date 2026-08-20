//! The ticket document model.
//!
//! A ticket is one markdown file: YAML frontmatter plus heading-delimited sections. The model holds
//! the file's bytes **verbatim** and describes its structure as byte spans into them. Nothing is
//! re-serialized, so rendering an unmodified ticket returns the original bytes by construction
//! rather than by careful formatting, and a mutation is a span replacement that leaves every other
//! byte — frontmatter comments, key order, blank lines, prose, trailing whitespace — untouched.
//!
//! That property is what the rest of the tool stands on: an agent appending one Build log line must
//! not silently reformat the file it appended to.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::contract::Owned;
use crate::errors::{Result, SkaldError};

/// A byte range into a ticket's source.
type Span = Range<usize>;

#[derive(Clone, Debug)]
pub struct Ticket {
    /// The filename without `.md`. The store, not the frontmatter, decides a ticket's identity.
    pub id: String,
    pub path: PathBuf,
    source: String,
    pub frontmatter: Frontmatter,
    sections: Vec<Section>,
}

#[derive(Clone, Debug, Default)]
pub struct Frontmatter {
    pub present: bool,
    /// An opening `---` with no closing delimiter. The block is then unparsed: guessing where it was
    /// meant to end would invent structure, and reporting it is `check`'s job.
    pub unterminated: bool,
    /// The whole block, both `---` delimiter lines included.
    block: Span,
    /// The region between the delimiters, where a new key is inserted by the write commands.
    #[allow(dead_code)]
    inner: Span,
    entries: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    /// 1-based line of the `key:` line. Recorded on the read path because `check` reports
    /// violations as navigable `file:line`, and the line is only knowable while parsing.
    #[allow(dead_code)]
    pub line: usize,
    /// The `key:` line plus every continuation line belonging to it.
    span: Span,
    /// Everything after the colon to the end of the key line, leading whitespace included. Empty
    /// width when the value is blank, so writing `key: value` is one replacement either way.
    value_span: Span,
}

impl Entry {
    /// The byte range a `set` replaces.
    pub fn value_range(&self) -> Span {
        self.value_span.clone()
    }

    /// The whole entry, continuation lines included — what replacing a block list has to replace.
    pub fn range(&self) -> Span {
        self.span.clone()
    }

    /// Where this entry's first line begins, for inserting a key before it.
    pub fn start(&self) -> usize {
        self.span.start
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// The key is present with nothing after the colon — an unfilled field, not a mistake.
    Empty,
    Scalar(String),
    List(Vec<String>),
    /// A nested mapping. skald does not own the shape of a store's nested keys, so it preserves and
    /// reports one without interpreting it.
    Nested,
}

/// A heading and everything beneath it, down to the next heading at the same or a shallower level.
///
/// Every heading level is indexed, not just `##`: the sections agents address most — `Intake`,
/// `Build log` — are `###` in the template, and a `##`-only index would make them unaddressable.
#[derive(Clone, Debug)]
pub struct Section {
    pub title: String,
    pub level: usize,
    /// Address form of the title. Sections resolve through this, so `Build log`, `build-log`, and
    /// `BUILD LOG` all name the same section — agents do not reproduce a heading exactly.
    pub slug: String,
    /// The heading line, its newline included.
    heading: Span,
    /// After the heading line to the start of the next same-or-shallower heading. Descendants are
    /// included, so addressing `Journal` addresses the whole journal.
    body: Span,
}

impl Ticket {
    pub fn parse(id: impl Into<String>, path: impl Into<PathBuf>, source: String) -> Self {
        let frontmatter = parse_frontmatter(&source);
        let body_start = match frontmatter.present && !frontmatter.unterminated {
            true => frontmatter.block.end,
            false => 0,
        };
        let sections = parse_sections(&source, body_start);
        Self {
            id: id.into(),
            path: path.into(),
            source,
            frontmatter,
            sections,
        }
    }

    pub fn read(id: impl Into<String>, path: &Path) -> Result<Self> {
        let id = id.into();
        let source = std::fs::read_to_string(path)
            .map_err(|source| SkaldError::ReadTicket(path.to_path_buf(), source))?;
        Ok(Self::parse(id, path, source))
    }

    /// The file's bytes, exactly as read.
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Plain titles in document order. `section_addresses` is what user-facing output uses; this is
    /// the shape assertions read best.
    #[cfg(test)]
    pub fn section_titles(&self) -> Vec<String> {
        self.sections
            .iter()
            .map(|section| section.title.clone())
            .collect()
    }

    /// Section names for a "did you mean" list. A section whose address is not simply its slugified
    /// title — a punctuation-only heading, or the second of two that collide — carries the address
    /// too, because otherwise the list names something the reader cannot then pass to `--section`.
    pub fn section_addresses(&self) -> Vec<String> {
        self.sections
            .iter()
            .map(|section| match slugify(&section.title) == section.slug {
                true => section.title.clone(),
                false => format!("{} (--section {})", section.title, section.slug),
            })
            .collect()
    }

    /// Resolve a section by title, slug, or any spelling that slugifies the same way.
    ///
    /// A name of pure punctuation slugifies to nothing and therefore matches nothing: every real
    /// section has a non-empty slug, so an unaddressable name is an error rather than a silent hit on
    /// whichever section happened to be first.
    pub fn section(&self, name: &str) -> Option<&Section> {
        let wanted = slugify(name);
        if wanted.is_empty() {
            return None;
        }
        // An alias for a section skald owns resolves to that section's real heading, so `ac` and
        // `Acceptance criteria` are one address. The heading stays readable in the file while the
        // short form stays typable in a command.
        let wanted = match Owned::from_address(&wanted) {
            Some(owned) => slugify(owned.heading()),
            None => wanted,
        };
        self.sections.iter().find(|section| section.slug == wanted)
    }

    /// 1-based line of a section's heading, so a violation in the body is navigable too.
    pub fn line_of_section(&self, section: &Section) -> usize {
        line_of(&self.source, section.heading.start)
    }

    /// A section's heading line plus its body, verbatim.
    pub fn section_text(&self, section: &Section) -> &str {
        &self.source[section.heading.start..section.body.end]
    }

    /// A section's body without its heading line, verbatim.
    pub fn section_body(&self, section: &Section) -> &str {
        &self.source[section.body.clone()]
    }

    pub fn entries(&self) -> &[Entry] {
        &self.frontmatter.entries
    }

    /// A frontmatter entry's lines, verbatim — the way back to the bytes for a value shape skald
    /// does not own, such as a nested mapping.
    pub fn entry_text(&self, entry: &Entry) -> &str {
        &self.source[entry.span.clone()]
    }

    /// Where a section's heading line starts and ends, for removing the heading itself.
    pub fn section_heading_range(&self, section: &Section) -> Span {
        section.heading.clone()
    }

    /// Where a new frontmatter key goes when no later key exists to anchor it: just before the
    /// closing delimiter.
    pub fn frontmatter_insert_position(&self) -> usize {
        self.frontmatter.inner.end
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.frontmatter
            .entries
            .iter()
            .find(|entry| entry.key == key)
    }

    /// The scalar text of a frontmatter key, or `None` when absent, empty, or not a scalar.
    pub fn scalar(&self, key: &str) -> Option<&str> {
        match &self.entry(key)?.value {
            Value::Scalar(value) => Some(value),
            _ => None,
        }
    }

    /// The items of a list-valued frontmatter key. A scalar counts as a one-item list so a store
    /// that writes `projects: skald` filters the same as one that writes a block list.
    pub fn list(&self, key: &str) -> Vec<&str> {
        match &self.entry(key).map(|entry| &entry.value) {
            Some(Value::List(items)) => items.iter().map(String::as_str).collect(),
            Some(Value::Scalar(value)) => vec![value.as_str()],
            _ => Vec::new(),
        }
    }
}

/// A pending set of span replacements against one ticket.
///
/// The mutation commands land in the write-commands slice; the primitive ships here because byte
/// preservation is the property they all depend on, and it is only testable against a change.
#[allow(dead_code)]
impl Ticket {
    pub fn edits(&self) -> Edits<'_> {
        Edits {
            ticket: self,
            replacements: Vec::new(),
        }
    }

    /// Replace a frontmatter key's value in place, leaving its line, order, and any comment around
    /// it untouched. `false` when the key is absent or holds a value this cannot express.
    ///
    /// A block list or nested mapping continues onto lines that `value_span` does not cover, so
    /// writing a scalar over one would splice a second value in beside the old one and leave invalid
    /// YAML behind. Refusing is the only safe answer: a caller that means to replace a multi-line
    /// value has to say so, and until it can, the ticket is not silently corrupted. An *inline* value
    /// — `projects: [a, b]` — lives entirely on the key line and is replaceable.
    pub fn set_value_edit<'a>(&'a self, edits: &mut Edits<'a>, key: &str, value: &str) -> bool {
        let Some(entry) = self.entry(key) else {
            return false;
        };
        if self.entry_text(entry).trim_end().contains('\n') {
            return false;
        }
        let text = match value.is_empty() {
            true => String::new(),
            false => format!(" {value}"),
        };
        edits.replace(entry.value_span.clone(), text);
        true
    }

    /// Replace a section's body wholesale, keeping its heading line.
    pub fn set_section_edit<'a>(&'a self, edits: &mut Edits<'a>, section: &Section, body: &str) {
        edits.replace(section.body.clone(), body.to_string());
    }

    /// Whether another section follows this one, so a caller can tell whether its body needs to end
    /// with a separating blank line.
    pub fn section_is_followed(&self, section: &Section) -> bool {
        section.body.end < self.source.len()
    }

    /// The line ending this file uses, so writes extend the file's own convention rather than
    /// sprinkling `\n` through a CRLF ticket.
    pub fn newline(&self) -> &'static str {
        match self.source.contains("\r\n") {
            true => "\r\n",
            false => "\n",
        }
    }

    /// Append one line to the end of a section's body.
    ///
    /// The body span is rewritten rather than inserted into, so the separators come out the same way
    /// every time: one blank line after the heading, the existing content verbatim, the new line, and
    /// one blank line before the next heading when there is one. Computing an insertion point instead
    /// meant the blank line that separated this section from the next got consumed as the one after
    /// the heading, and the sections ran together.
    pub fn append_to_section_edit<'a>(
        &'a self,
        edits: &mut Edits<'a>,
        section: &Section,
        line: &str,
    ) {
        let newline = self.newline();
        // Verbatim, so existing entries keep their own bytes; only the blank lines around them are
        // normalized.
        let existing = self.source[section.body.clone()].trim_matches(['\n', '\r']);

        let mut body = String::from(newline);
        if !existing.is_empty() {
            body.push_str(existing);
            body.push_str(newline);
        }
        body.push_str(line.trim());
        body.push_str(newline);
        if self.section_is_followed(section) {
            body.push_str(newline);
        }
        edits.replace(section.body.clone(), body);
    }
}

#[allow(dead_code)]
pub struct Edits<'a> {
    ticket: &'a Ticket,
    replacements: Vec<(Span, String)>,
}

#[allow(dead_code)]
impl<'a> Edits<'a> {
    pub fn replace(&mut self, span: Span, text: String) {
        // Spans come from the parser today, but they will come from callers as the write commands
        // land. A span that is out of range or lands mid-codepoint would otherwise surface as an
        // opaque slice panic far from the mistake.
        debug_assert!(
            self.ticket.source.is_char_boundary(span.start)
                && self.ticket.source.is_char_boundary(span.end)
                && span.start <= span.end,
            "ticket edit span {span:?} is not a character-aligned range of the source"
        );
        self.replacements.push((span, text));
    }

    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }

    /// Splice every replacement into the source in one pass. Bytes outside the replaced spans are
    /// copied, never regenerated.
    ///
    /// # Panics
    /// If two replacements overlap. That is a caller bug, not user input: the outcome would depend
    /// on application order, and silently picking one is how a mutation corrupts a ticket.
    pub fn render(mut self) -> String {
        self.replacements.sort_by_key(|(span, _)| span.start);
        let source = &self.ticket.source;
        let mut out = String::with_capacity(source.len());
        let mut cursor = 0;
        for (span, text) in &self.replacements {
            assert!(
                span.start >= cursor,
                "overlapping ticket edits at byte {}",
                span.start
            );
            out.push_str(&source[cursor..span.start]);
            out.push_str(text);
            cursor = span.end;
        }
        out.push_str(&source[cursor..]);
        out
    }
}

fn parse_frontmatter(source: &str) -> Frontmatter {
    let Some(open_len) = open_delimiter_len(source) else {
        return Frontmatter::default();
    };

    let Some((inner, close_end)) = close_delimiter(source, open_len) else {
        return Frontmatter {
            present: true,
            unterminated: true,
            ..Frontmatter::default()
        };
    };

    Frontmatter {
        present: true,
        unterminated: false,
        entries: parse_entries(source, inner.clone()),
        block: 0..close_end,
        inner,
    }
}

fn open_delimiter_len(source: &str) -> Option<usize> {
    let rest = source.strip_prefix("---")?;
    if rest.starts_with("\r\n") {
        return Some(5);
    }
    rest.starts_with('\n').then_some(4)
}

/// The span between the delimiters, plus the offset just past the closing line.
fn close_delimiter(source: &str, open_len: usize) -> Option<(Span, usize)> {
    let mut offset = open_len;
    for line in source[open_len..].split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            return Some((open_len..offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

fn parse_entries(source: &str, inner: Span) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut line_number = line_of(source, inner.start);
    let mut offset = inner.start;

    for line in source[inner.clone()].split_inclusive('\n') {
        let end = offset + line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim();

        // A continuation belongs to the key above it. Indented text and list items extend that
        // key's span; anything else — a comment, a blank line — is neither part of an entry nor
        // ours to interpret, and survives untouched because it is never inside a replaced span.
        let indented = content.starts_with([' ', '\t']);
        if let Some(entry) = entries.last_mut()
            && (indented || trimmed.starts_with("- ") || trimmed == "-")
        {
            match list_item(trimmed) {
                Some(item) => match &mut entry.value {
                    Value::List(items) => items.push(item.to_string()),
                    Value::Empty => entry.value = Value::List(vec![item.to_string()]),
                    _ => entry.value = Value::Nested,
                },
                // An indented `key: value` under a key is a nested mapping.
                None if matches!(entry.value, Value::Empty) && indented => {
                    entry.value = Value::Nested;
                }
                None => {}
            }
            entry.span.end = end;
            offset = end;
            line_number += 1;
            continue;
        }

        if let Some((key, after_colon_at)) = split_key_absolute(content, offset) {
            let (raw_value, _comment) = split_trailing_comment(&content[after_colon_at - offset..]);
            let value = raw_value.trim();
            entries.push(Entry {
                key: key.to_string(),
                value: match value.is_empty() {
                    true => Value::Empty,
                    false => match inline_list(value) {
                        Some(items) => Value::List(items),
                        None => Value::Scalar(unquote(value).to_string()),
                    },
                },
                line: line_number,
                span: offset..end,
                // From just after the colon to the end of the value, so one replacement both fills
                // an empty key (`pr:` → `pr: url`) and clears a filled one back to exactly `pr:`.
                // It stops before any trailing comment, which a `set` therefore leaves alone.
                value_span: after_colon_at..after_colon_at + raw_value.trim_end().len(),
            });
        }

        offset = end;
        line_number += 1;
    }

    entries
}

/// `key: value` at column zero → the key and the absolute offset just past the colon.
fn split_key(content: &str) -> Option<(&str, usize)> {
    if content.starts_with([' ', '\t']) || content.trim_start().starts_with('#') {
        return None;
    }
    let colon = content.find(':')?;
    let key = &content[..colon];
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key, colon + 1))
}

fn split_key_absolute(content: &str, offset: usize) -> Option<(&str, usize)> {
    split_key(content).map(|(key, relative)| (key, offset + relative))
}

/// Split everything after a `key:` into its value and any trailing comment.
///
/// YAML's rule: in an unquoted scalar a `#` starts a comment only when whitespace precedes it. That
/// distinction is load-bearing here — the ticket template annotates its enum keys
/// (`mode: direct  # ENUM: direct | orchestrated`), while `pr:` holds URLs whose `#fragment` is part
/// of the value.
fn split_trailing_comment(after_colon: &str) -> (&str, &str) {
    let trimmed = after_colon.trim_start();
    let value_start = after_colon.len() - trimmed.len();

    // A quoted scalar may contain `#`, so start scanning past its closing quote.
    let mut search = match trimmed.chars().next() {
        Some(quote @ ('"' | '\'')) => match trimmed[1..].find(quote) {
            Some(close) => value_start + close + 2,
            None => return (after_colon, ""),
        },
        _ => value_start,
    };

    while let Some(hash) = after_colon[search..].find('#') {
        let at = search + hash;
        if at == value_start || after_colon[..at].ends_with([' ', '\t']) {
            return (&after_colon[..at], &after_colon[at..]);
        }
        search = at + 1;
    }

    (after_colon, "")
}

fn list_item(trimmed: &str) -> Option<&str> {
    let item = trimmed
        .strip_prefix("- ")
        .or_else(|| (trimmed == "-").then_some(""))?;
    Some(unquote(item.trim()))
}

/// `[a, b]` only. `None` means the value is not a list skald can read, so it stays a scalar and
/// `check` gets to have an opinion about it.
fn inline_list(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    Some(
        inner
            .split(',')
            .map(|item| unquote(item.trim()))
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|value| value.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// Four decisions in here are each a way a heading can be wrongly seen or wrongly missed, and a
/// missed heading is worse than a missed ticket: the section is unaddressable, and an append to its
/// predecessor silently lands inside it.
fn parse_sections(source: &str, body_start: usize) -> Vec<Section> {
    let mut headings: Vec<Section> = Vec::new();
    // Which marker opened the current fence, not merely "in a fence" — a `~~~` line inside a
    // ```-fence must not close it, or the real closing ``` re-opens one and every heading after it
    // disappears. Build logs carry fenced markdown, so this is ordinary content, not a corner case.
    let mut fence: Option<&str> = None;
    let mut offset = body_start;

    for line in source[body_start..].split_inclusive('\n') {
        let end = offset + line.len();
        let trimmed = line.trim();

        if let Some(marker) = fence_marker(trimmed) {
            match fence {
                Some(open) if open == marker => fence = None,
                Some(_) => {}
                None => fence = Some(marker),
            }
            offset = end;
            continue;
        }

        // A `#` inside a fence is a shell comment or a Rust attribute, not structure. Four or more
        // leading spaces make an indented code block, whose `#` is likewise content.
        if fence.is_none()
            && leading_spaces(line) < 4
            && let Some((level, title)) = heading_line(trimmed)
        {
            headings.push(Section {
                slug: slugify(title),
                title: title.to_string(),
                level,
                heading: offset..end,
                body: end..source.len(),
            });
        }

        offset = end;
    }

    // A section ends where the next same-or-shallower heading begins; deeper headings are part of it.
    for index in 0..headings.len() {
        let level = headings[index].level;
        if let Some(next) = headings[index + 1..]
            .iter()
            .find(|section| section.level <= level)
        {
            headings[index].body.end = next.heading.start;
        }
    }

    disambiguate_slugs(&mut headings);
    headings
}

/// Every section gets a distinct, non-empty slug, because a slug is an address: two sections sharing
/// one makes the second unreachable, and a `set-section` would silently overwrite the first instead.
fn disambiguate_slugs(headings: &mut [Section]) {
    let mut seen: Vec<String> = Vec::with_capacity(headings.len());
    for heading in headings.iter_mut() {
        // A heading of pure punctuation (`## ---`) slugifies to nothing. Without a fallback, *any*
        // name a caller passes that also slugifies to nothing — `!!!`, `🎉` — would match it.
        if heading.slug.is_empty() {
            heading.slug = "section".to_string();
        }
        if seen.contains(&heading.slug) {
            let base = std::mem::take(&mut heading.slug);
            heading.slug = (2..)
                .map(|suffix| format!("{base}-{suffix}"))
                .find(|candidate| !seen.contains(candidate))
                .expect("an unbounded counter always yields an unused suffix");
        }
        seen.push(heading.slug.clone());
    }
}

/// ` ``` ` or `~~~`, the marker itself. An info string (` ```sh `) is part of the opening line.
fn fence_marker(trimmed: &str) -> Option<&'static str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| trimmed.starts_with(marker))
}

fn leading_spaces(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn heading_line(trimmed: &str) -> Option<(usize, &str)> {
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let text = trimmed[level..].strip_prefix(' ')?.trim();
    (!text.is_empty()).then_some((level, text))
}

/// Lowercase, non-alphanumerics collapsed to single dashes. `Build log`, `build-log`, and
/// `"Build Log:"` all address the same section.
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// 1-based line number of a byte offset.
pub fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::{Ticket, Value, slugify};

    /// Deliberately awkward: a frontmatter comment, an unusual key order, a blank line inside the
    /// block, CRLF-free but with trailing spaces, a fenced `#`, and no trailing newline discipline.
    const AWKWARD: &str = "\
---
# written by hand, order matters
id: ask-2026-08-20-core-read
phase: build
pr:
projects:
  - skald
  - dotfiles/agents

updated: 2026-08-20
metadata:
  type: project
---

# Title

Intro.

## Journal

### Build log

- 2026-08-20: first.

```sh
# not a heading
```

### Open questions

- None.
";

    fn ticket() -> Ticket {
        Ticket::parse(
            "ask-2026-08-20-core-read",
            "/store/t.md",
            AWKWARD.to_string(),
        )
    }

    #[test]
    fn an_unmodified_ticket_renders_byte_for_byte() {
        let ticket = ticket();
        assert_eq!(ticket.source(), AWKWARD);
        assert_eq!(ticket.edits().render(), AWKWARD);
    }

    #[test]
    fn frontmatter_keeps_order_types_and_line_numbers() {
        let ticket = ticket();
        let keys: Vec<&str> = ticket.entries().iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["id", "phase", "pr", "projects", "updated", "metadata"]
        );
        assert_eq!(ticket.scalar("phase"), Some("build"));
        assert_eq!(ticket.entry("pr").unwrap().value, Value::Empty);
        assert_eq!(ticket.list("projects"), vec!["skald", "dotfiles/agents"]);
        assert_eq!(ticket.entry("metadata").unwrap().value, Value::Nested);
        // Line 1 is the opening `---`, line 2 the comment, so `id` is line 3.
        assert_eq!(ticket.entry("id").unwrap().line, 3);
    }

    #[test]
    fn setting_a_value_changes_only_that_value() {
        let ticket = ticket();
        let mut edits = ticket.edits();
        assert!(ticket.set_value_edit(&mut edits, "phase", "review"));
        let rendered = edits.render();

        assert_eq!(rendered, AWKWARD.replace("phase: build", "phase: review"));
        // The comment, the blank line inside the block, and key order all survive.
        assert!(rendered.contains("# written by hand, order matters"));
        assert!(rendered.contains("  - dotfiles/agents\n\nupdated:"));
    }

    #[test]
    fn filling_an_empty_value_and_clearing_it_again_round_trips() {
        let ticket = ticket();
        let mut edits = ticket.edits();
        ticket.set_value_edit(&mut edits, "pr", "https://example.test/pr/1");
        let filled = edits.render();
        assert!(filled.contains("pr: https://example.test/pr/1\n"));

        let refilled = Ticket::parse("t", "/store/t.md", filled);
        let mut edits = refilled.edits();
        refilled.set_value_edit(&mut edits, "pr", "");
        assert_eq!(edits.render(), AWKWARD);
    }

    #[test]
    fn sections_index_every_level_and_ignore_fenced_hashes() {
        let ticket = ticket();
        assert_eq!(
            ticket.section_titles(),
            vec!["Title", "Journal", "Build log", "Open questions"]
        );
        // `Journal` owns its descendants, so addressing it addresses the whole journal.
        assert!(
            ticket
                .section_body(ticket.section("Journal").unwrap())
                .contains("### Build log")
        );
        assert!(
            !ticket
                .section_body(ticket.section("Build log").unwrap())
                .contains("Open questions")
        );
    }

    #[test]
    fn sections_resolve_by_any_spelling_that_slugifies_the_same() {
        let ticket = ticket();
        for name in ["Build log", "build-log", "BUILD LOG", "build_log"] {
            assert_eq!(ticket.section(name).unwrap().title, "Build log", "{name}");
        }
        assert!(ticket.section("Nonexistent").is_none());
    }

    #[test]
    fn appending_lands_at_the_end_of_the_section_not_after_its_trailing_blank_lines() {
        let ticket = ticket();
        let section = ticket.section("build-log").unwrap();
        let mut edits = ticket.edits();
        ticket.append_to_section_edit(&mut edits, section, "\n- 2026-08-20: second.");
        let rendered = edits.render();

        // The fenced block is the last content in Build log, so the append goes after it — and
        // still above the blank line that separates the section from Open questions, which is what
        // keeps repeated appends from widening the gap each time.
        assert!(rendered.contains("```\n- 2026-08-20: second.\n\n### Open questions"));
        // Everything outside the append is identical.
        assert_eq!(rendered.replace("\n- 2026-08-20: second.", ""), AWKWARD);
    }

    #[test]
    fn replacing_a_section_body_keeps_its_heading_and_its_neighbours() {
        let ticket = ticket();
        let section = ticket.section("open-questions").unwrap();
        let mut edits = ticket.edits();
        ticket.set_section_edit(&mut edits, section, "\n- Resolved: nothing open.\n");
        let rendered = edits.render();

        assert!(rendered.contains("### Open questions\n\n- Resolved: nothing open.\n"));
        assert!(rendered.contains("- 2026-08-20: first."));
    }

    #[test]
    fn two_edits_in_one_pass_do_not_disturb_each_other() {
        let ticket = ticket();
        let mut edits = ticket.edits();
        ticket.set_value_edit(&mut edits, "phase", "review");
        ticket.set_value_edit(&mut edits, "updated", "2026-08-21");
        let rendered = edits.render();
        assert_eq!(
            rendered,
            AWKWARD
                .replace("phase: build", "phase: review")
                .replace("updated: 2026-08-20", "updated: 2026-08-21")
        );
    }

    #[test]
    #[should_panic(expected = "overlapping ticket edits")]
    fn overlapping_edits_are_a_caller_bug_not_a_silent_corruption() {
        let ticket = ticket();
        let mut edits = ticket.edits();
        edits.replace(0..10, "a".to_string());
        edits.replace(5..15, "b".to_string());
        edits.render();
    }

    #[test]
    fn a_ticket_with_no_frontmatter_is_still_readable() {
        let source = "# Title\n\n## Notes\n\nprose\n";
        let ticket = Ticket::parse("t", "/store/t.md", source.to_string());
        assert!(!ticket.frontmatter.present);
        assert!(ticket.entries().is_empty());
        assert_eq!(ticket.section_titles(), vec!["Title", "Notes"]);
        assert_eq!(ticket.edits().render(), source);
    }

    #[test]
    fn an_unterminated_block_is_reported_and_left_whole() {
        let source = "---\nid: t\n\n# Title\n";
        let ticket = Ticket::parse("t", "/store/t.md", source.to_string());
        assert!(ticket.frontmatter.present);
        assert!(ticket.frontmatter.unterminated);
        assert!(ticket.entries().is_empty());
        assert_eq!(ticket.edits().render(), source);
    }

    #[test]
    fn crlf_frontmatter_round_trips() {
        let source = "---\r\nid: t\r\nphase: build\r\n---\r\n\r\n# Title\r\n";
        let ticket = Ticket::parse("t", "/store/t.md", source.to_string());
        assert_eq!(ticket.scalar("phase"), Some("build"));
        let mut edits = ticket.edits();
        ticket.set_value_edit(&mut edits, "phase", "review");
        assert_eq!(
            edits.render(),
            source.replace("phase: build", "phase: review")
        );
    }

    #[test]
    fn inline_and_quoted_values_read_the_same_as_block_ones() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "---\nprojects: [skald, \"dotfiles\"]\ntitle: 'quoted'\n---\n".to_string(),
        );
        assert_eq!(ticket.list("projects"), vec!["skald", "dotfiles"]);
        assert_eq!(ticket.scalar("title"), Some("quoted"));
    }

    #[test]
    fn a_trailing_comment_is_not_part_of_the_value_and_survives_a_set() {
        // The shape the ticket template actually ships: aligned values with an annotation after.
        let source = "---\nmode: direct       # ENUM: direct | orchestrated\nid:                # <ID>\n---\n";
        let ticket = Ticket::parse("t", "/store/t.md", source.to_string());
        assert_eq!(ticket.scalar("mode"), Some("direct"));
        assert_eq!(ticket.entry("id").unwrap().value, Value::Empty);

        let mut edits = ticket.edits();
        ticket.set_value_edit(&mut edits, "mode", "orchestrated");
        let rendered = edits.render();
        assert!(rendered.contains("mode: orchestrated       # ENUM: direct | orchestrated\n"));
        assert!(rendered.contains("id:                # <ID>\n"));
    }

    #[test]
    fn a_url_fragment_is_part_of_the_value_because_no_space_precedes_it() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "---\npr: https://example.test/pr/1#files\n---\n".to_string(),
        );
        assert_eq!(ticket.scalar("pr"), Some("https://example.test/pr/1#files"));
    }

    #[test]
    fn a_multi_line_value_is_refused_rather_than_half_replaced() {
        let source =
            "---\nprojects:\n  - a\n  - b\nmetadata:\n  type: project\nphase: build\n---\n";
        let ticket = Ticket::parse("t", "/store/t.md", source.to_string());
        let mut edits = ticket.edits();

        // A scalar written over a block list would leave `projects: zzz` *above* `  - a`, which is
        // invalid YAML and a silently corrupted ticket.
        assert!(!ticket.set_value_edit(&mut edits, "projects", "zzz"));
        assert!(!ticket.set_value_edit(&mut edits, "metadata", "flat"));
        assert!(!ticket.set_value_edit(&mut edits, "absent", "x"));
        assert!(edits.is_empty());
        assert_eq!(edits.render(), source);

        // An inline list lives on the key line, so replacing it is expressible.
        let inline = Ticket::parse("t", "/store/t.md", "---\nprojects: []\n---\n".to_string());
        let mut edits = inline.edits();
        assert!(inline.set_value_edit(&mut edits, "projects", "[skald]"));
        assert_eq!(edits.render(), "---\nprojects: [skald]\n---\n");
    }

    #[test]
    fn a_nested_fence_marker_does_not_close_the_outer_fence() {
        // A Build log quoting fenced markdown is ordinary content. Tracking only "in a fence" would
        // let the inner `~~~` close it and the real closing ``` re-open one, hiding every heading
        // that follows.
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "## One\n\n```\n~~~\n## not a heading\n```\n\n## Two\n\ntail\n".to_string(),
        );
        assert_eq!(ticket.section_titles(), vec!["One", "Two"]);
    }

    #[test]
    fn an_indented_code_block_is_not_a_heading() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "## Real\n\n    ## indented code\n\n## Also real\n".to_string(),
        );
        assert_eq!(ticket.section_titles(), vec!["Real", "Also real"]);
    }

    #[test]
    fn a_punctuation_only_heading_is_addressable_and_does_not_swallow_other_names() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "## ---\n\ndashes\n\n## Real\n\nprose\n".to_string(),
        );
        assert_eq!(ticket.section("section").unwrap().title, "---");
        // A name that also slugifies to nothing must not silently resolve to it.
        for name in ["", "!!!", "🎉"] {
            assert!(ticket.section(name).is_none(), "{name}");
        }
    }

    #[test]
    fn an_owned_sections_alias_resolves_to_its_real_heading() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "## Acceptance criteria\n\n- a criterion\n\n## Log\n\n- an entry\n".to_string(),
        );
        for name in ["ac", "AC", "acceptance-criteria", "Acceptance criteria"] {
            assert_eq!(
                ticket.section(name).map(|section| section.title.as_str()),
                Some("Acceptance criteria"),
                "{name}"
            );
        }
        // An alias resolves to the owned section or to nothing; it never falls through to a
        // same-named ordinary section.
        let without = Ticket::parse("t", "/store/t.md", "## Notes\n\nprose\n".to_string());
        assert!(without.section("ac").is_none());
    }

    #[test]
    fn a_did_you_mean_list_names_addresses_a_caller_can_actually_pass() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "## Notes\n\na\n\n## notes\n\nb\n\n## ---\n\nc\n".to_string(),
        );
        assert_eq!(
            ticket.section_addresses(),
            vec![
                "Notes".to_string(),
                "notes (--section notes-2)".to_string(),
                "--- (--section section)".to_string(),
            ]
        );
    }

    #[test]
    fn sections_sharing_a_slug_each_get_their_own_address() {
        let ticket = Ticket::parse(
            "t",
            "/store/t.md",
            "## Notes\n\nfirst\n\n## notes\n\nsecond\n\n## NOTES\n\nthird\n".to_string(),
        );
        let slugs: Vec<&str> = ticket
            .sections()
            .iter()
            .map(|section| section.slug.as_str())
            .collect();
        assert_eq!(slugs, vec!["notes", "notes-2", "notes-3"]);
        assert_eq!(
            ticket
                .section_body(ticket.section("notes-2").unwrap())
                .trim(),
            "second"
        );
    }

    #[test]
    fn slugs_normalize_punctuation_and_case() {
        assert_eq!(slugify("Build log"), "build-log");
        assert_eq!(
            slugify("Spec — acceptance criteria"),
            "spec-acceptance-criteria"
        );
        assert_eq!(
            slugify("Checkpoints (memory boundaries)"),
            "checkpoints-memory-boundaries"
        );
    }
}
