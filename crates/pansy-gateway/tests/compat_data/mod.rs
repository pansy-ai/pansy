//! The samples `compat.rs` runs: each thing in all four protocols, in the
//! order Anthropic, Chat, Responses, Gemini, as agents and vendors write it.

mod answers;
mod requests;

use pansy_gateway::ir::{Request, Response};

pub use answers::ANSWERS;
pub use requests::REQUESTS;

/// Something an agent asks.
pub struct Ask {
    pub title: &'static str,
    /// The request in each protocol; `None`: the protocol can't ask it.
    pub samples: [Option<&'static str>; 4],
    /// Why a vendor of the protocol can't be asked it at all.
    pub no_vendor: [Option<&'static str>; 4],
    /// What the vendor's body must show, by protocol.
    pub wire: [&'static [(&'static str, &'static str)]; 4],
    /// The thing is something the vendor must *not* get: nothing to read back.
    pub wire_only: bool,
    pub check: fn(&Request) -> Result<(), String>,
}

/// Something a vendor answers, whole and streamed.
pub struct Answer {
    pub title: &'static str,
    pub whole: [&'static str; 4],
    pub stream: [&'static str; 4],
    /// What the agent's whole answer must show, by protocol.
    pub wire: [&'static [(&'static str, &'static str)]; 4],
    pub check: fn(&Response) -> Result<(), String>,
}

pub fn need(ok: bool, what: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}
