//! The ticket contract, compiled in.
//!
//! skald is opinionated: it owns the field set, the status vocabulary, and the two sections a ticket
//! must have. There is deliberately no per-store schema file. A store that declared its own contract
//! would let every store agree with itself by construction, which is the opposite of what a contract
//! is for — and an opinionated tool that declares nothing is just a validator.
//!
//! The purpose the shape serves is **resume**: an agent picking up a ticket it did not start should
//! learn where the work is, what "done" means, and what happened so far, in one read.
//!
//! This module is a declaration rather than logic, and it declares the contract *whole* — including
//! the parts `check` and the write commands are the first to read. Stating it in one place is the
//! point: a rule split between here and its enforcement site is a rule that drifts. Hence the
//! blanket allow, which is scoped to this module alone.
#![allow(dead_code)]

/// A field in a ticket's frontmatter, in the order `new` writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Status,
    /// Free text. Present means the ticket is parked; absent means it is running.
    ///
    /// Deliberately *not* a status value. A `status: paused` would destroy which phase the ticket was
    /// paused from, which is the same defect as writing `reviewing (pending Ian)` — the qualifier is
    /// information, and there would be nowhere for it to go. Blocked is simply a pause whose reason
    /// is external, so it needs no state of its own.
    Paused,
    Repos,
    Branch,
    Link,
    Pr,
    Parent,
    Created,
    Updated,
}

/// Every field, in the order they are written to a new ticket.
pub const FIELDS: [Field; 10] = [
    Field::Title,
    Field::Status,
    Field::Paused,
    Field::Repos,
    Field::Branch,
    Field::Link,
    Field::Pr,
    Field::Parent,
    Field::Created,
    Field::Updated,
];

/// What a field's value has to look like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Text,
    /// One bare value from [`STATUSES`].
    Status,
    List,
    /// An absolute `http(s)` URL.
    Url,
    /// Another ticket's id, in this store.
    TicketId,
    /// `YYYY-MM-DD`.
    Date,
}

impl Field {
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Status => "status",
            Self::Paused => "paused",
            Self::Repos => "repos",
            Self::Branch => "branch",
            Self::Link => "link",
            Self::Pr => "pr",
            Self::Parent => "parent",
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        FIELDS.into_iter().find(|field| field.key() == key)
    }

    pub fn shape(self) -> Shape {
        match self {
            Self::Title | Self::Paused | Self::Branch => Shape::Text,
            Self::Status => Shape::Status,
            Self::Repos => Shape::List,
            Self::Link | Self::Pr => Shape::Url,
            Self::Parent => Shape::TicketId,
            Self::Created | Self::Updated => Shape::Date,
        }
    }

    /// The key must be present, even when its value is empty. Presence and emptiness are separate:
    /// `pr:` is present and empty for most of a ticket's life.
    pub fn required(self) -> bool {
        matches!(
            self,
            Self::Title | Self::Status | Self::Repos | Self::Created | Self::Updated
        )
    }

    /// `key:` with nothing after it is acceptable. False for the fields a ticket is meaningless
    /// without.
    pub fn allows_empty(self) -> bool {
        !matches!(
            self,
            Self::Title | Self::Status | Self::Created | Self::Updated
        )
    }

    /// Written by skald, never by hand, so they cannot drift from what actually happened.
    pub fn is_managed(self) -> bool {
        matches!(self, Self::Created | Self::Updated)
    }

    /// One line of guidance, for `skald docs` and for error messages.
    pub fn description(self) -> &'static str {
        match self {
            Self::Title => "What this ticket is, in one line.",
            Self::Status => "Which phase owns the ticket now.",
            Self::Paused => "Why the ticket should not be worked on. Present means parked.",
            Self::Repos => "Repos this work touches.",
            Self::Branch => "Where the code lives, so a resuming agent can find it.",
            Self::Link => "The upstream ticket or issue, if this came from one.",
            Self::Pr => "The pull request, once there is one.",
            Self::Parent => "The ticket this one was split from or follows up.",
            Self::Created => "When the ticket was created. Managed by skald.",
            Self::Updated => "When the ticket last changed. Managed by skald.",
        }
    }
}

/// The status vocabulary.
///
/// A status names the phase that **owns** the ticket, and it advances the moment the previous phase
/// finishes — not when work starts. So `Building` means "refining is done and build is the
/// outstanding work", whether or not anyone has begun. That is the only reading an agent can apply
/// without guessing at intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Refining,
    Building,
    Reviewing,
    Done,
    Cancelled,
}

pub const STATUSES: [Status; 5] = [
    Status::Refining,
    Status::Building,
    Status::Reviewing,
    Status::Done,
    Status::Cancelled,
];

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Refining => "refining",
            Self::Building => "building",
            Self::Reviewing => "reviewing",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        STATUSES.into_iter().find(|status| status.name() == value)
    }

    /// No status follows a terminal one. Follow-up work is a new ticket with `parent:` pointing back,
    /// which keeps a finished ticket's record of what "done" meant intact.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }

    /// Movement among the three live states is free in **any** direction. Whether a review finding is
    /// a small fix (stay in `Reviewing`), a large hole in the implementation (back to `Building`), or
    /// a problem with the AC itself (back to `Refining`) is judgment, and a tool that guessed would
    /// be wrong often enough to be worked around.
    pub fn may_move_to(self, next: Self) -> bool {
        !self.is_terminal() || self == next
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Refining => {
                "Working out what this is and what done means. The only status in which the acceptance criteria may be written."
            }
            Self::Building => "Refining is finished; implementation is the outstanding work.",
            Self::Reviewing => "Building is finished; review is the outstanding work.",
            Self::Done => "Finished. Terminal.",
            Self::Cancelled => "Abandoned deliberately. Terminal.",
        }
    }
}

/// A section skald owns. Everything else in a ticket body is free-form prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owned {
    /// The contract. Frozen outside [`Status::Refining`].
    AcceptanceCriteria,
    /// The append-only trail that makes a stopped ticket resumable.
    Log,
}

pub const OWNED_SECTIONS: [Owned; 2] = [Owned::AcceptanceCriteria, Owned::Log];

impl Owned {
    /// The heading skald writes.
    pub fn heading(self) -> &'static str {
        match self {
            Self::AcceptanceCriteria => "Acceptance criteria",
            Self::Log => "Log",
        }
    }

    /// Every address that resolves to this section. `ac` is here because it is what a human types and
    /// what fits in a command, while the heading stays readable in the file.
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::AcceptanceCriteria => &["acceptance-criteria", "ac"],
            Self::Log => &["log"],
        }
    }

    /// Resolve a user-supplied section name to an owned section, if it names one.
    pub fn from_address(slug: &str) -> Option<Self> {
        OWNED_SECTIONS
            .into_iter()
            .find(|owned| owned.aliases().contains(&slug))
    }

    /// Whether a whole-body replacement is allowed.
    ///
    /// The log is append-only: rewriting an audit trail destroys the property that makes it worth
    /// reading when resuming, which is the log's entire job.
    pub fn allows_replace(self) -> bool {
        !matches!(self, Self::Log)
    }
}

#[cfg(test)]
mod tests {
    use super::{FIELDS, Field, Owned, Shape, Status};

    #[test]
    fn every_field_round_trips_through_its_key() {
        for field in FIELDS {
            assert_eq!(Field::from_key(field.key()), Some(field), "{field:?}");
        }
        assert_eq!(Field::from_key("phase"), None);
        assert_eq!(Field::from_key("projects"), None);
        // The field set is closed, so the old vocabulary is a violation rather than an extension.
        assert_eq!(Field::from_key("id"), None);
    }

    #[test]
    fn required_and_empty_are_separate_questions() {
        // `pr` is present and empty for most of a ticket's life, and `title` is meaningless empty.
        assert!(!Field::Pr.required());
        assert!(Field::Pr.allows_empty());
        assert!(Field::Title.required());
        assert!(!Field::Title.allows_empty());
        assert!(Field::Repos.required());
        assert!(Field::Repos.allows_empty());
    }

    #[test]
    fn managed_fields_are_exactly_the_dates() {
        let managed: Vec<&str> = FIELDS
            .into_iter()
            .filter(|field| field.is_managed())
            .map(Field::key)
            .collect();
        assert_eq!(managed, vec!["created", "updated"]);
    }

    #[test]
    fn only_terminal_statuses_refuse_to_move() {
        for from in [Status::Refining, Status::Building, Status::Reviewing] {
            for to in [
                Status::Refining,
                Status::Building,
                Status::Reviewing,
                Status::Done,
                Status::Cancelled,
            ] {
                assert!(from.may_move_to(to), "{from:?} -> {to:?}");
            }
        }
        // Backwards among the live states is legal on purpose: review finding a large hole sends the
        // ticket back to building, and a bad AC sends it back to refining.
        assert!(Status::Reviewing.may_move_to(Status::Refining));

        for from in [Status::Done, Status::Cancelled] {
            assert!(from.may_move_to(from), "a no-op set is not a move");
            for to in [Status::Refining, Status::Building, Status::Reviewing] {
                assert!(!from.may_move_to(to), "{from:?} -> {to:?}");
            }
        }
        assert!(!Status::Done.may_move_to(Status::Cancelled));
    }

    #[test]
    fn a_decorated_status_is_not_a_status() {
        assert_eq!(Status::parse("reviewing"), Some(Status::Reviewing));
        // The case the whole tool exists for.
        assert_eq!(Status::parse("reviewing (pending Ian)"), None);
        assert_eq!(Status::parse("Reviewing"), None);
        assert_eq!(Status::parse("reviewed"), None);
        assert_eq!(Status::parse(""), None);
    }

    #[test]
    fn owned_sections_resolve_by_alias_and_the_log_cannot_be_replaced() {
        assert_eq!(Owned::from_address("ac"), Some(Owned::AcceptanceCriteria));
        assert_eq!(
            Owned::from_address("acceptance-criteria"),
            Some(Owned::AcceptanceCriteria)
        );
        assert_eq!(Owned::from_address("log"), Some(Owned::Log));
        assert_eq!(Owned::from_address("notes"), None);

        assert!(Owned::AcceptanceCriteria.allows_replace());
        assert!(!Owned::Log.allows_replace());
    }

    #[test]
    fn shapes_match_what_each_field_carries() {
        assert_eq!(Field::Repos.shape(), Shape::List);
        assert_eq!(Field::Pr.shape(), Shape::Url);
        assert_eq!(Field::Parent.shape(), Shape::TicketId);
        assert_eq!(Field::Updated.shape(), Shape::Date);
    }
}
