//! The licence of everything this application ships with.
//!
//! Every permissive licence in the tree asks for the same thing, and it is the only thing they ask
//! for: the notice travels with the code. So it travels — collected at build time by
//! `scripts/collect-licences.sh`, committed, and embedded here.
//!
//! Embedded rather than fetched or read from beside the binary: a notice that can go missing is a
//! notice that will, and an installation with the code and without the licence is the one state
//! this must not have.

use serde::Serialize;
use ts_rs::TS;

use crate::commands::error::{CommandError, ErrorCode};

/// The list as it was collected.
///
/// Written by `scripts/collect-licences.sh`, committed, and rebuilt by `scripts/preflight.sh`, which
/// fails on any difference: a dependency added without its notice ships without its licence.
pub(crate) const COLLECTED: &str = include_str!("../../licences.json");

/// One notice, and what it covers.
#[derive(Debug, Clone, Serialize, TS, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct NoticeDto {
    /// The licence by name, as its own packages declare it.
    pub licence: String,
    /// The notice itself, word for word. Never summarised: a summary is not a licence.
    pub text: String,
    /// Everything this notice covers, by name.
    ///
    /// Grouped by the text rather than by the package, because that is what a notice is: sixty
    /// crates under one MIT with one copyright line is one notice, and listing it sixty times
    /// would be a screen nobody reaches the end of.
    pub used: Vec<String>,
}

/// Everything this application ships with, and what each of it is licensed under.
#[tauri::command]
pub async fn licences() -> Result<Vec<NoticeDto>, CommandError> {
    serde_json::from_str(COLLECTED).map_err(|_| {
        CommandError::new(
            ErrorCode::Internal,
            "The licence list shipped with this build could not be read.",
        )
    })
}
