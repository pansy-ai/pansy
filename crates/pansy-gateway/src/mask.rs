//! Masking in the gateway: a request's body is masked before it is routed,
//! so a vendor gets placeholders whether the request passes through or is
//! translated, and the answer the agent gets has the values back. What was
//! masked goes on record (`pansy_core::masking`), without the values.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use pansy_core::masking::{self, Change, Record, Value};
use pansy_mask::{Action, Hit, Masker, Options};

/// This install's masker: one for the process, so a value masked in one
/// request has the same placeholder in the next and is put back in any
/// answer. None when its key could not be read or made.
pub fn masker() -> Option<Arc<Masker>> {
    static MASKER: OnceLock<Option<Arc<Masker>>> = OnceLock::new();
    MASKER
        .get_or_init(|| match masking::key() {
            Ok(key) => Some(Arc::new(Masker::new(&key))),
            Err(e) => {
                tracing::warn!("masking is off: its key: {e:#}");
                None
            }
        })
        .clone()
}

/// A request masked.
pub struct Masked {
    /// The body to send, when masking changed it.
    pub body: Option<Vec<u8>>,
    /// The record's key, when anything was found.
    pub key: Option<u64>,
    /// The kinds that held the request back, when any did.
    pub blocked: Vec<String>,
}

/// Masks `body` as `options` say, and records what was found.
pub fn request(
    masker: &Masker,
    body: &[u8],
    options: &Options,
    agent: &str,
    provider: &str,
    model: &str,
) -> Masked {
    let started = Instant::now();
    let masked = masker.mask_json(body, options);
    if masked.hits.is_empty() {
        return Masked {
            body: None,
            key: None,
            blocked: Vec::new(),
        };
    }
    let took = started.elapsed().as_micros() as u64;
    let mut blocked: Vec<String> = masked
        .hits
        .iter()
        .filter(|h| h.action == Action::Block)
        .map(|h| h.kind.clone())
        .collect();
    blocked.dedup();
    let record = Record {
        key: masking::next_key(),
        t: chrono::Utc::now(),
        agent: agent.to_owned(),
        provider: provider.to_owned(),
        model: model.to_owned(),
        n: masked
            .hits
            .iter()
            .filter(|h| h.action != Action::Warn)
            .count(),
        size: body.len(),
        values: values(&masked.hits),
        blocked: masked.blocked,
        took,
        back: BTreeMap::new(),
    };
    let changes = masked
        .changes
        .into_iter()
        .map(|c| Change {
            path: c.path,
            before: c.before,
            after: c.after,
        })
        .collect();
    if let Err(e) = masking::append(&record, changes) {
        tracing::warn!("recording masking: {e:#}");
    }
    Masked {
        body: (masked.body.as_slice() != body).then_some(masked.body),
        key: Some(record.key),
        blocked,
    }
}

/// Hits by placeholder: each value once, with the places it was found.
fn values(hits: &[Hit]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for hit in hits {
        if let Some(v) = out.iter_mut().find(|v| v.p == hit.placeholder) {
            v.n += 1;
            if v.w.len() < 5 && !hit.path.is_empty() && !v.w.contains(&hit.path) {
                v.w.push(hit.path.clone());
            }
            continue;
        }
        out.push(Value {
            p: hit.placeholder.clone(),
            k: hit.kind.clone(),
            b: hit.by.clone(),
            h: hit.head.clone(),
            l: hit.len,
            n: 1,
            a: match hit.action {
                Action::Mask => String::new(),
                Action::Warn => "warn".to_owned(),
                Action::Block => "block".to_owned(),
            },
            w: if hit.path.is_empty() {
                Vec::new()
            } else {
                vec![hit.path.clone()]
            },
        });
    }
    out
}
