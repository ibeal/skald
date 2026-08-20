//! `skald docs` — the guidance an agent can reach without a file read.
//!
//! This exists because of the permission deny. Once the ticket directory is denied, an agent that
//! cannot work out the interface has no fallback: it cannot read the store, and it cannot read the
//! repo it is not in. `skald docs` is the recovery path, so it is compiled into the binary rather
//! than left on disk where it might not be reachable.

use crate::cli::DocsTopic;

pub fn render(topic: DocsTopic) -> &'static str {
    match topic {
        DocsTopic::Agent => include_str!("../docs/agent-usage.md"),
    }
}
