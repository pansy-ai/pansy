//! A line to the user's own webhook when a thread waits for them or ends:
//! ntfy, Bark, a bot. It says the thread's title and what happened, never
//! what was said (phone-protocol.md). Push through pansy's own relay comes
//! with pansy Connect.

use crate::store::Thread;

pub fn approval(thread: &Option<Thread>, what: &str) {
    if let Some(t) = thread {
        send(t, &format!("waits for your approval: {what}"));
    }
}

pub fn turn_finished(thread: &Option<Thread>, status: &str) {
    let Some(t) = thread else { return };
    let what = match status {
        "completed" => "finished",
        "failed" => "failed",
        _ => return,
    };
    send(t, what);
}

fn send(t: &Thread, what: &str) {
    let Ok(settings) = crate::config::load() else {
        return;
    };
    let url = settings.webhook.trim().to_owned();
    if url.is_empty() {
        return;
    }
    let title = if t.title.is_empty() {
        "pansy"
    } else {
        &t.title
    }
    .to_owned();
    let body = format!("{title}: {what}");
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        // ntfy reads a Title header and the body as the message; others take
        // the body alone, or JSON they ignore the rest of.
        let sent = client
            .post(&url)
            .header("Title", ascii(&title))
            .header("Tags", "pansy")
            .body(body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        if let Err(e) = sent {
            tracing::warn!("the webhook: {e}");
        }
    });
}

/// A header's value: non-ASCII text percent-free but readable is not
/// allowed in headers, so it is dropped there (the body keeps it).
fn ascii(s: &str) -> String {
    let kept: String = s
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .collect();
    if kept.trim().is_empty() {
        "pansy".into()
    } else {
        kept
    }
}
