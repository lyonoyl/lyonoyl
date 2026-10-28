//! Pure domain of Lyonoyl Mail: concepts, invariants and decisions.
//!
//! This crate knows nothing about SQLite, Tauri, IMAP or JSON. Everything here can be tested
//! without a database, a network or a display — which is exactly why the rules that protect the
//! user's mail live here and nowhere else.

pub mod account;
pub mod account_group;
pub mod as_asked;
pub mod attachment;
pub mod error;
pub mod folder;
pub mod mailbox_name;
pub mod merge;
pub mod message;
pub mod notice;
pub mod operation;
pub mod outgoing;
pub mod preferences;
pub mod provider;
pub mod search;
pub mod sync;
pub mod thread;
pub mod version;
pub mod window;
pub mod xoauth2;
