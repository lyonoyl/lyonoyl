//! Asking whether a newer version is out, and installing one.
//!
//! Thin, like every command here: a DTO in, a use case, an error out. What decides anything is
//! next door in `application/updating.rs`, and what fetches and verifies is two floors down.

use std::collections::BTreeMap;

use serde::Serialize;
use tauri::Emitter;
use ts_rs::TS;

use crate::application::error::UseCaseError;
use crate::application::settings::GetPreferences;
use crate::application::updating::{seconds_now, InstallTheUpdate, LookForAnUpdate};
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::infrastructure::updates::asking_the_project::{
    this_build_can_be_updated, AskingTheProject,
};
use crate::ports::updates::Offered;
use crate::AppState;

/// A version that is out, as the window needs it.
///
/// The token the adapter uses to fetch it does not cross: a window that held it would be a window
/// that could be asked to install something else.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct OfferedDto {
    /// As three numbers, the way the manifest named it.
    pub version: String,
    /// How large the download is. A number rather than a big integer: an application is megabytes,
    /// and megabytes fit a JavaScript number exactly.
    #[ts(type = "number")]
    pub bytes: u64,
    /// What changed, as the release said it, under the name of each catalogue it wrote it for.
    pub notes: BTreeMap<String, String>,
}

impl OfferedDto {
    fn of(offered: &Offered) -> Self {
        Self {
            version: offered.version.to_string(),
            bytes: offered.bytes,
            notes: offered.notes.clone(),
        }
    }
}

/// How far a download has got, on its way to the window.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct HowFarDto {
    #[ts(type = "number")]
    pub so_far: u64,
    /// What the whole is, where the host said. Absent when it did not, which is when a bar cannot
    /// be drawn and a count of what has arrived is all there is to show.
    #[ts(type = "number | null")]
    pub whole: Option<u64>,
}

/// What changed in a version, for the screen that shows it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct WhatChangedDto {
    pub version: String,
    /// As the release wrote it, under the name of each catalogue it wrote it for; the window shows
    /// the one in the language the person chose. Plain text: what a message may not do to this
    /// window, a release note may not either.
    pub notes: BTreeMap<String, String>,
}

/// What pressing *Install* turned out to do.
///
/// Two outcomes, and telling them apart is the point. The command asks the project again rather than
/// installing what a window was shown minutes ago — so *there is nothing newer after all* is an
/// ordinary answer, not a failure. Answering it the same way as a finished install told a person
/// their application had been replaced when nothing had been downloaded at all.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum InstalledDto {
    /// It came down, its seal held, and it is in place. The application has to be restarted.
    Swapped,
    /// The project no longer offers anything newer. Nothing was fetched and nothing was replaced.
    NothingNewer,
}

/// The event the window listens for while an update comes down.
pub const COMING: &str = "update:coming";

/// And the one that says the download is over and the swap has started.
///
/// Its own event rather than a full bar left on screen: the two are different waits, and the second
/// is the one where the application is about to be replaced under somebody. A bar sitting at the end
/// for however long the unpacking takes is a bar that has stopped meaning anything.
pub const PUTTING: &str = "update:putting";

/// Whether a newer version is out.
///
/// Asked two ways and the difference matters. On a timer it is silent: a machine that is offline
/// says *nothing is out*, because somebody reading their mail did not ask and has nothing to do
/// about it. Asked by a person pressing a button, it answers what happened — a control that reports
/// nothing is a control that looks broken.
#[tauri::command]
pub async fn look_for_an_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    aloud: bool,
) -> Result<Option<OfferedDto>, CommandError> {
    if !this_build_can_be_updated() {
        return Ok(None);
    }

    // What the person answered, read before a copy looks on its own. Storage that will not open
    // says nothing was agreed to, and nothing is looked for: the careful side again.
    let allowed = match state.storage.profile() {
        Ok(pool) => tauri::async_runtime::spawn_blocking(move || {
            GetPreferences::new(adapters::preferences(pool)).execute()
        })
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|preferences| preferences.look_for_updates().looks_on_its_own()),
        Err(_) => false,
    };

    let project = the_project(&app)?;

    tauri::async_runtime::spawn_blocking(move || {
        let looking = LookForAnUpdate::new(project);

        match aloud {
            true => looking
                .because_somebody_asked()
                .map(|found| found.as_ref().map(OfferedDto::of)),
            false => Ok(looking
                .execute(allowed, seconds_now())
                .as_ref()
                .map(OfferedDto::of)),
        }
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "The update check could not be run."))?
    .map_err(CommandError::from)
}

/// Brings the offered version down, verifies it, and puts it in place.
///
/// Answers when the application has to be restarted to use what was installed. Progress goes to the
/// window as it happens rather than at the end: a download of tens of megabytes with nothing on
/// screen is a dead control.
#[tauri::command]
pub async fn install_the_update(
    app: tauri::AppHandle,
    window: tauri::Window,
) -> Result<InstalledDto, CommandError> {
    let project = the_project(&app)?;

    tauri::async_runtime::spawn_blocking(move || -> Result<InstalledDto, UseCaseError> {
        let looking = LookForAnUpdate::new(&project);

        // Asked again rather than carried from the earlier answer. What a window holds is what a
        // person was shown minutes ago, and installing what a stale answer named is installing
        // something nobody was offered.
        let Some(offered) = looking.because_somebody_asked()? else {
            return Ok(InstalledDto::NothingNewer);
        };

        let installing = InstallTheUpdate::new(&project);

        let staged = installing.bring_it_down(&offered, &mut |so_far, whole| {
            let _ = window.emit(COMING, HowFarDto { so_far, whole });
            true
        })?;

        let _ = window.emit(PUTTING, ());

        installing.put_it_in_place(&staged, &offered.notes)?;

        Ok(InstalledDto::Swapped)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "The update could not be installed."))?
    .map_err(CommandError::from)
}

/// What changed in the version now running, where nobody has read it yet.
///
/// `None` on every ordinary start. It answers something exactly once: the first start after an
/// update this application installed itself, and only while the note it kept is about the version
/// that is actually running.
#[tauri::command]
pub async fn what_changed(app: tauri::AppHandle) -> Result<Option<WhatChangedDto>, CommandError> {
    let project = the_project(&app)?;

    // Asked of the use case that writes it, because the note is what installing leaves behind.
    Ok(InstallTheUpdate::new(project)
        .what_changed()
        .map(|(version, notes)| WhatChangedDto { version, notes }))
}

/// Read, and therefore not shown again.
///
/// Called by the cross that leaves the screen rather than by opening it. Marking it read on open
/// would lose it for somebody who was interrupted before reading a word — invariant 20 says leaving
/// a screen is the screen's own business, and this is what that screen does on the way out.
#[tauri::command]
pub async fn changes_seen(app: tauri::AppHandle) -> Result<(), CommandError> {
    InstallTheUpdate::new(the_project(&app)?).changes_seen();
    Ok(())
}

/// Starts the application again, so the version that was just put in place is the one running.
///
/// **Not a reload of the window.** The swap replaced the bundle on disk and threw the old one away;
/// the process that did it is still the old binary, and reloading what it draws changes nothing
/// about that. A person told the update was installed, who then pressed a button that reloaded a
/// page, would go on running the version they were told they had left — and the screen that says
/// what changed, which compares the note against the version running, could never appear.
///
/// Nothing comes back, because nothing here returns: the process is replaced.
#[tauri::command]
pub async fn restart_into_the_new_version(app: tauri::AppHandle) -> Result<(), CommandError> {
    app.restart()
}

/// The updater this asks, as the composition root builds it.
fn the_project(app: &tauri::AppHandle) -> Result<AskingTheProject, CommandError> {
    crate::bootstrap::updates::the_updater(app)
        .map_err(|why| CommandError::new(ErrorCode::Internal, why))
}
