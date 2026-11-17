//! pansy's data, shared by the gateway, the command line and the window:
//! providers with their accounts and models, the subscriptions signed in
//! to, routing groups, profiles, settings, the usage log and what masking
//! kept from vendors, all kept as files under [`paths::home`].

pub mod config;
pub mod fsutil;
pub mod group;
pub mod manage;
pub mod masking;
pub mod models;
pub mod paths;
pub mod plugin;
pub mod presets;
pub mod profile;
pub mod provider;
pub mod settings;
pub mod subscription;
pub mod usage;

/// pansy's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
