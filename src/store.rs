//! Resolving the active ticket store and the tickets inside it.

use std::path::PathBuf;

use crate::errors::{Result, STORE_VAR, SkaldError};
use crate::schema::Schema;
use crate::ticket::Ticket;

pub struct Store {
    pub root: PathBuf,
    pub schema: Option<Schema>,
}

/// The result of reading a whole store: the tickets, and the failures that did not stop the rest.
#[derive(Default)]
pub struct Listing {
    pub tickets: Vec<Ticket>,
    pub unreadable: Vec<SkaldError>,
}

impl Store {
    /// Resolve the store from `$SKALD_STORE`.
    ///
    /// There is exactly one active store per invocation, and no fallback of any kind: no default
    /// path, no current directory, no upward search. Writing a ticket into the wrong store is a
    /// worse outcome than refusing to run, and per-directory switching (direnv) already covers the
    /// case a store list would serve.
    pub fn resolve() -> Result<Self> {
        let raw = match std::env::var_os(STORE_VAR) {
            Some(raw) if !raw.is_empty() => PathBuf::from(raw),
            _ => return Err(SkaldError::StoreUnset),
        };
        Self::at(raw)
    }

    pub fn at(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() {
            return Err(SkaldError::StoreRelative(root));
        }
        let metadata = std::fs::metadata(&root).map_err(|source| match source.kind() {
            // "Does not exist" and "is not a directory" are different mistakes with different
            // fixes, so they get different messages.
            std::io::ErrorKind::NotFound => SkaldError::StoreMissing(root.clone()),
            _ => SkaldError::StoreUnreadable(root.clone(), source),
        })?;
        if !metadata.is_dir() {
            return Err(SkaldError::StoreNotADirectory(root));
        }
        // Readability is checked here rather than at first use, so the error names the store instead
        // of surfacing later as a confusingly empty list.
        std::fs::read_dir(&root)
            .map_err(|source| SkaldError::StoreUnreadable(root.clone(), source))?;

        let schema = Schema::read(&root)?;
        Ok(Self { root, schema })
    }

    /// The store's schema, or the one clear error a store without a declared contract earns on an
    /// operation that needs one.
    #[allow(dead_code)] // Consumed by `check` and the write commands.
    pub fn require_schema(&self) -> Result<&Schema> {
        self.schema
            .as_ref()
            .ok_or_else(|| SkaldError::SchemaMissing(Schema::path(&self.root)))
    }

    pub fn path_for(&self, id: &str) -> Result<PathBuf> {
        let id = id.strip_suffix(".md").unwrap_or(id);
        // An id is a name within one store, so a separator in it is either a mistake or an attempt
        // to reach outside the store. Both deserve the same refusal.
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.starts_with('.') {
            return Err(SkaldError::TicketIdNotAName(id.to_string()));
        }
        Ok(self.root.join(format!("{id}.md")))
    }

    /// Ticket ids in the store, sorted, so every listing is deterministic.
    pub fn ids(&self) -> Result<Vec<String>> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.root)
            .map_err(|source| SkaldError::StoreUnreadable(self.root.clone(), source))?
            .filter_map(|entry| entry.ok())
            // `symlink_metadata`, so a link out of the store is not listed as one of its tickets —
            // the same containment `ticket` enforces on the way in.
            .filter(|entry| {
                std::fs::symlink_metadata(entry.path()).is_ok_and(|metadata| metadata.is_file())
            })
            .filter_map(|entry| ticket_id(&entry.file_name().to_string_lossy()))
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Every ticket in the store, plus the ones that could not be read.
    ///
    /// One unreadable file — bad encoding, no permission — must not take the other twenty with it.
    /// Failing the whole listing would be the inverse of the invariant that makes a broken ticket
    /// visible: instead of one bad row you would lose every good one.
    pub fn tickets(&self) -> Result<Listing> {
        let mut listing = Listing::default();
        for id in self.ids()? {
            let path = self.root.join(format!("{id}.md"));
            match Ticket::read(id, &path) {
                Ok(ticket) => listing.tickets.push(ticket),
                Err(error) => listing.unreadable.push(error),
            }
        }
        Ok(listing)
    }

    /// Load one ticket, resolving the id with or without its `.md` extension.
    pub fn ticket(&self, id: &str) -> Result<Ticket> {
        let path = self.path_for(id)?;
        let id = id.strip_suffix(".md").unwrap_or(id).to_string();
        // `symlink_metadata` does not follow the link, which is the point: `path_for` refuses an id
        // that names a path out of the store, and a symlink is the other way to leave it.
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => Ticket::read(id, &path),
            Ok(_) => Err(SkaldError::TicketNotAFile(path)),
            // A bare "not found" sends the reader back to guessing filenames, which is exactly what
            // the store abstraction exists to stop.
            Err(_) => Err(SkaldError::UnknownTicket {
                candidates: self.near_matches(&id)?,
                id,
            }),
        }
    }

    fn near_matches(&self, id: &str) -> Result<Vec<String>> {
        let needle = id.to_lowercase();
        Ok(self
            .ids()?
            .into_iter()
            .filter(|candidate| {
                let candidate = candidate.to_lowercase();
                candidate.contains(&needle) || needle.contains(&candidate)
            })
            .take(5)
            .collect())
    }

    /// Reject a filter on a key this store's schema does not declare. A store with no schema has no
    /// declared keys to check against, and reads are allowed to work without one.
    pub fn check_filter_key(&self, key: &str) -> Result<()> {
        let Some(schema) = &self.schema else {
            return Ok(());
        };
        match schema.declares(key) {
            true => Ok(()),
            false => Err(SkaldError::UndeclaredFilterKey {
                key: key.to_string(),
                declared: schema.declared_keys(),
            }),
        }
    }
}

/// `ask-2026-08-20-core-read.md` → the id.
///
/// Three named exclusions, not a heuristic: dotfiles are store metadata (`.schema.toml`),
/// `_`-prefixed files are templates, and a store's own `README.md` documents the store. Everything
/// else in the directory is a ticket even if it is a bad one — a malformed ticket that quietly
/// vanished from `list` and `check` would be the exact failure this tool exists to catch.
fn ticket_id(file_name: &str) -> Option<String> {
    if file_name.starts_with('.')
        || file_name.starts_with('_')
        || file_name.eq_ignore_ascii_case("README.md")
    {
        return None;
    }
    Some(file_name.strip_suffix(".md")?.to_string())
}

/// A store directory laid out in a temp dir, for tests.
#[cfg(test)]
pub fn fixture(label: &str, files: &[(&str, &str)]) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("skald-{label}-{unique}"));
    std::fs::create_dir_all(&root).unwrap();
    for (name, contents) in files {
        std::fs::write(root.join(name), contents).unwrap();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::{Store, fixture, ticket_id};
    use crate::errors::SkaldError;

    const ONE: &str = "---\nid: one\nphase: build\nprojects:\n  - skald\n---\n\n# One\n";
    const TWO: &str = "---\nid: two\nphase: review\nprojects:\n  - dotfiles\n---\n\n# Two\n";

    fn store() -> (std::path::PathBuf, Store) {
        let root = fixture(
            "store",
            &[
                ("one.md", ONE),
                ("two.md", TWO),
                ("_TICKET_TEMPLATE.md", "---\nid:\n---\n"),
                (
                    ".schema.toml",
                    "[keys.id]\ntype = \"string\"\n[keys.phase]\ntype = \"enum\"\nvalues = [\"build\", \"review\"]\n",
                ),
                ("notes.txt", "not a ticket"),
            ],
        );
        let store = Store::at(root.clone()).unwrap();
        (root, store)
    }

    #[test]
    fn templates_metadata_and_non_markdown_are_not_tickets() {
        let (root, store) = store();
        assert_eq!(store.ids().unwrap(), vec!["one", "two"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_id_resolves_with_or_without_the_extension() {
        let (root, store) = store();
        assert_eq!(store.ticket("one").unwrap().source(), ONE);
        assert_eq!(store.ticket("one.md").unwrap().source(), ONE);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_ticket_suggests_what_the_store_actually_holds() {
        let (root, store) = store();
        let error = store.ticket("on").unwrap_err();
        assert!(matches!(
            &error,
            SkaldError::UnknownTicket { candidates, .. } if candidates == &["one".to_string()]
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_id_that_is_a_path_is_refused_rather_than_followed() {
        let (root, store) = store();
        for id in ["../secrets", "sub/one", ".schema.toml"] {
            assert!(
                matches!(store.ticket(id), Err(SkaldError::TicketIdNotAName(_))),
                "{id}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filtering_on_an_undeclared_key_names_the_declared_ones() {
        let (root, store) = store();
        assert!(store.check_filter_key("phase").is_ok());
        assert!(matches!(
            store.check_filter_key("service"),
            Err(SkaldError::UndeclaredFilterKey { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_store_without_a_schema_still_reads_and_still_filters() {
        let root = fixture("schemaless", &[("one.md", ONE)]);
        let store = Store::at(root.clone()).unwrap();
        assert!(store.schema.is_none());
        assert_eq!(store.ticket("one").unwrap().source(), ONE);
        // No declared contract means no declared keys to reject a filter against.
        assert!(store.check_filter_key("anything").is_ok());
        // But an operation that needs the contract says so once, clearly.
        assert!(matches!(
            store.require_schema(),
            Err(SkaldError::SchemaMissing(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_relative_store_is_refused_because_it_would_resolve_against_the_cwd() {
        assert!(matches!(
            Store::at(std::path::PathBuf::from("tickets")),
            Err(SkaldError::StoreRelative(_))
        ));
    }

    #[test]
    fn one_unreadable_ticket_does_not_take_the_listing_with_it() {
        let root = fixture("unreadable", &[("one.md", ONE), ("two.md", TWO)]);
        // Invalid UTF-8, which `read_to_string` refuses.
        std::fs::write(root.join("bad.md"), [0x2d, 0x2d, 0x2d, 0x0a, 0xff, 0xfe]).unwrap();
        let store = Store::at(root.clone()).unwrap();

        let listing = store.tickets().unwrap();
        let ids: Vec<&str> = listing
            .tickets
            .iter()
            .map(|ticket| ticket.id.as_str())
            .collect();
        assert_eq!(ids, vec!["one", "two"]);
        // Named, not dropped: losing every good ticket to one bad one is the inverse of the
        // invariant that makes a broken ticket visible in the first place.
        assert_eq!(listing.unreadable.len(), 1);
        assert!(listing.unreadable[0].to_string().contains("bad.md"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_symlink_is_not_a_way_out_of_the_store() {
        let root = fixture("symlink", &[("one.md", ONE)]);
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), "---\nid: secret\n---\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("escape.md")).unwrap();

        let store = Store::at(root.clone()).unwrap();
        // `path_for` refuses an id that spells a path out of the store; a link is the other way out,
        // and skald is meant to be the only way in.
        assert!(matches!(
            store.ticket("escape"),
            Err(SkaldError::TicketNotAFile(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_store_reads_differently_from_one_that_is_not_a_directory() {
        let root = fixture("missing", &[("one.md", ONE)]);
        assert!(matches!(
            Store::at(root.join("nope")),
            Err(SkaldError::StoreMissing(_))
        ));
        assert!(matches!(
            Store::at(root.join("one.md")),
            Err(SkaldError::StoreNotADirectory(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ticket_id_strips_the_extension_only_for_real_tickets() {
        assert_eq!(ticket_id("one.md").as_deref(), Some("one"));
        assert_eq!(ticket_id("notes.txt"), None);
        assert_eq!(ticket_id(".schema.toml"), None);
        assert_eq!(ticket_id("_TICKET_TEMPLATE.md"), None);
        assert_eq!(ticket_id("README.md"), None);
        // A ticket with no frontmatter at all is still a ticket, so `check` gets to say so.
        assert_eq!(ticket_id("broken.md").as_deref(), Some("broken"));
    }
}
