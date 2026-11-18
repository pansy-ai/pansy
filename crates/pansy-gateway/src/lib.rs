//! pansy's gateway: one local endpoint that speaks OpenAI Chat, OpenAI
//! Responses, Anthropic Messages and Gemini, sends each request to the
//! provider and account its model names, translating between protocols
//! where the two differ, and moves on to the next account or group member
//! when one cannot answer.

pub mod count;
pub mod ir;
pub mod mask;
pub mod plugins;
pub mod prompt;
pub mod proto;
pub mod route;
pub mod server;
pub mod sse;
pub mod trace;
mod upstream;

pub use server::{Gateway, serve};
