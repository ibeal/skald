//! Best-effort delivery of externally observable ticket state transitions.

use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use crate::ticket::Ticket;

const ATTEMPTS: usize = 3;

/// Send a transition after its ticket write has committed. A broken receiver must never turn a
/// completed local mutation into a failed command, so delivery failure is only reported on stderr.
pub fn emit(ticket: &Ticket, previous_status: &str, status: &str, url: Option<&str>) {
    let Some(url) = url else {
        return;
    };
    let payload = payload(&ticket.id, previous_status, status);

    if let Some(error) = deliver(&url, &payload) {
        eprintln!(
            "warning: state-change webhook delivery failed after {ATTEMPTS} attempts: {error}"
        );
    }
}

fn deliver(url: &str, payload: &serde_json::Value) -> Option<String> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(error) => return Some(format!("could not initialize client: {error}")),
    };
    retry(|| match client.post(url).json(payload).send() {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => Err(format!("receiver returned {}", response.status())),
        Err(error) => Err(error.to_string()),
    })
}

fn retry(send: impl FnMut() -> Result<(), String>) -> Option<String> {
    let mut send = send;
    let mut last_error = None;
    for _ in 0..ATTEMPTS {
        match send() {
            Ok(()) => return None,
            Err(error) => last_error = Some(error),
        }
    }
    last_error.or_else(|| Some("unknown error".to_string()))
}

fn payload(ticket_id: &str, previous_status: &str, status: &str) -> serde_json::Value {
    let occurred_at = jiff::Timestamp::now().to_string();
    let event_id = Uuid::new_v4().to_string();
    json!({
        "version": 1,
        "event_id": event_id,
        "occurred_at": &occurred_at,
        "type": "skald.ticket.transitioned",
        "ticket_id": ticket_id,
        "previous_status": previous_status,
        "status": status,
        "ticket_revision": &occurred_at,
    })
}

/// The externally visible state is `paused` precisely when a pause reason is present.
pub fn derived_status(status: &str, paused: Option<&str>) -> String {
    match paused.filter(|reason| !reason.is_empty()) {
        Some(_) => "paused".to_string(),
        None => status.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{derived_status, payload, retry};
    use uuid::Uuid;

    #[test]
    fn pause_reason_derives_paused_state() {
        assert_eq!(derived_status("building", None), "building");
        assert_eq!(derived_status("building", Some("waiting")), "paused");
        assert_eq!(derived_status("building", Some("")), "building");
    }

    #[test]
    fn payload_has_the_public_transition_contract() {
        let payload = payload("ticket-1", "designing", "building");
        assert_eq!(payload["version"], 1);
        assert_eq!(payload["type"], "skald.ticket.transitioned");
        assert_eq!(payload["ticket_id"], "ticket-1");
        assert_eq!(payload["previous_status"], "designing");
        assert_eq!(payload["status"], "building");
        assert_eq!(payload["occurred_at"], payload["ticket_revision"]);
        Uuid::parse_str(payload["event_id"].as_str().unwrap()).unwrap();
    }

    #[test]
    fn retries_reuse_one_event_id() {
        let event = payload("ticket-1", "designing", "building");
        let mut attempted = Vec::new();
        assert!(
            retry(|| {
                attempted.push(event.clone());
                Err("receiver unavailable".to_string())
            })
            .is_some()
        );
        assert_eq!(attempted.len(), 3);
        let event_ids: Vec<_> = attempted
            .iter()
            .map(|payload| payload["event_id"].clone())
            .collect();
        assert!(event_ids.windows(2).all(|pair| pair[0] == pair[1]));
    }
}
