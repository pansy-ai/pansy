//! pansy's privacy masking, a port of magpie's. What the gateway sends a
//! vendor has its secrets (API keys, private keys, tokens, passwords) and,
//! when asked, personal data and the user's own words swapped for
//! placeholders like `{{API_KEY_k3v9x2mq}}`; what the vendor says back has
//! them swapped back, streams included, so the agent and the user see the
//! real thing while the vendor never does.
//!
//! A placeholder is the same for the same value every time: the first 8
//! letters of the base32 of an HMAC-SHA256 of its kind and value, keyed by
//! the [`Masker`]'s key. A conversation's history reads the same turn after
//! turn, so what a vendor cached of it stays good. The values are held in
//! memory only. A value once masked is masked wherever a request has it,
//! whatever words are around it; a value the user allows always goes as
//! written; a kind the user warns of goes as written and is reported, and
//! one the user blocks is masked and holds the request back.
//!
//! Nothing here reads a file or the network: the caller gives the key.

mod crypto;
mod ed25519;
mod fieldvalue;
mod finder;
mod ids;
mod json;
mod known;
mod mask;
mod more;
mod options;
mod personal;
mod re;
mod restore;
mod secrets;
mod store;
mod stream;
mod text;
mod words;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;

pub use options::{
    Action, CATEGORIES, Category, Options, PERSONAL_KINDS, Rule, RuleKind, SECRET_KINDS, WORD_KIND,
    kind_name, rule_kind,
};
pub use stream::StreamRestorer;

/// One value found. It never holds the value itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// The placeholder it went as, or would have, for one sent as written.
    pub placeholder: String,
    pub kind: String,
    /// The finder that found it: "rule", "yours" (a user's rule), "word"
    /// or "known" (a value masked before).
    pub by: String,
    /// Its first character, or a key's prefix (sk-, ghp_).
    pub head: String,
    /// Its length in characters.
    pub len: usize,
    pub action: Action,
    /// Where in a JSON body, as messages[0].content; "" for a text.
    pub path: String,
}

/// One text the masking changed, for showing the user before and after.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    pub path: String,
    pub before: String,
    pub after: String,
}

/// A text masked.
#[derive(Debug, Clone, PartialEq)]
pub struct Masked {
    pub text: String,
    pub hits: Vec<Hit>,
}

/// A request body masked.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskedBody {
    /// The body, byte for byte as it came when nothing was masked.
    pub body: Vec<u8>,
    pub hits: Vec<Hit>,
    pub changes: Vec<Change>,
    /// A value of a kind the user blocks was found: the request is not to
    /// be sent.
    pub blocked: bool,
}

/// Masks and puts back. One is meant to serve every request of a run, so a
/// value masked in one request is put back in the answer to any, and its
/// key is meant to be kept across runs, so placeholders stay the same.
pub struct Masker {
    key: Vec<u8>,
    store: RwLock<store::Store>,
    sent: Mutex<mask::Sent>,
}

impl Masker {
    /// A masker whose placeholders are keyed by `key` (32 random bytes,
    /// kept by the caller).
    pub fn new(key: &[u8]) -> Masker {
        Masker {
            key: key.to_vec(),
            store: RwLock::new(store::Store::new()),
            sent: Mutex::new(mask::Sent::default()),
        }
    }

    /// Masks a text.
    pub fn mask_text(&self, text: &str, o: &Options) -> Masked {
        if !o.wanted() {
            return Masked {
                text: text.to_owned(),
                hits: Vec::new(),
            };
        }
        let cx = mask::Cx::new(o);
        let mut out = mask::Out::default();
        let text = self.mask_str(text, o, &cx, &mut out, false);
        Masked {
            text,
            hits: out.hits,
        }
    }

    /// Masks a request body's texts, in any of the four protocols
    /// (Anthropic Messages, OpenAI Chat Completions and Responses, Gemini):
    /// every string a model reads, a tool call's arguments as JSON in a
    /// string too, but not ids, models, roles, types, tool names,
    /// signatures, encrypted content, encoded data or an image's URL. In a
    /// tool call's arguments a field named for a secret (api_key, password)
    /// is masked whatever its value looks like. The body comes back byte
    /// for byte as it was when nothing was masked, and as it is when it
    /// isn't JSON.
    pub fn mask_json(&self, body: &[u8], o: &Options) -> MaskedBody {
        self.mask_body(body, o)
    }

    /// Puts the values back in a text, and counts each placeholder put
    /// back.
    pub fn restore(&self, text: &str) -> (String, BTreeMap<String, usize>) {
        let mut put = BTreeMap::new();
        let out = self.restore_into(text, false, Some(&mut put));
        (out, put)
    }

    /// Puts the values back in a whole response body: in each of its
    /// strings, escaped in a tool call's arguments as JSON there needs; in
    /// all of it where it isn't JSON.
    pub fn restore_json(&self, body: &[u8]) -> (Vec<u8>, BTreeMap<String, usize>) {
        let mut put = BTreeMap::new();
        let out = self.restore_body(body, &mut put);
        (out, put)
    }

    /// The value behind a placeholder, while it is held.
    pub fn value(&self, placeholder: &str) -> Option<String> {
        self.store
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values
            .get(placeholder)
            .cloned()
    }

    /// Says some value is masked, and may be put back.
    pub fn holding(&self) -> bool {
        !self
            .store
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values
            .is_empty()
    }

    /// Puts values back in a stream as it comes. The restorer holds its own
    /// `Arc` of this masker, so it can live in an async stream:
    /// `let mut r = masker.stream();` then `r.push(&chunk)` for each chunk
    /// and `r.finish()` at the end, sending on what each gives back.
    pub fn stream(self: &Arc<Self>) -> StreamRestorer {
        StreamRestorer::new(Arc::clone(self))
    }
}
