//! One line about the whole workspace: is the mail current, and did what was asked for land.

use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::commands::error::{CommandError, ErrorCode};
use crate::read_model::activity::ActivityReadModel;
use crate::AppState;

/// Why passes are not finishing, in the terms a person can act on.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum WhyItStoppedDto {
    Offline,
    SignIn,
    Storage,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct HowThingsAreDto {
    /// Unix seconds. Declared as a number rather than a big integer: seconds fit a JavaScript
    /// number exactly.
    #[ts(type = "number | null")]
    pub last_success_at: Option<i64>,
    /// Present only while no account is finishing a pass: one signed-out account among four does
    /// not make the mail on screen stale.
    pub stopped_by: Option<WhyItStoppedDto>,
    pub accounts_failing: u32,
    /// Instructions no server has taken yet, and late: tried at least once, or waiting longer than
    /// a push and a round take. One on its way as usual is not counted.
    pub waiting: u32,
    /// Messages somebody pressed send on that no server has taken yet, counted the same way.
    pub sending: u32,
    /// Instructions that stopped and are waiting for a person.
    pub stopped: u32,
    /// Messages somebody pressed send on that are back in the drafts, because the outbox will not
    /// try them again. A server would not take them, would not let the account sign in, or could
    /// not be reached on any of the outbox's tries; or nobody knows whether one has them, because
    /// the conversation ended after the message was handed over or a round took them and never
    /// wrote down what became of them. Each counted for as long as the draft it came back as is
    /// there, and not once that draft is sent again or thrown away.
    pub came_back: u32,
}

/// What a person asked to be read from a server, and how far it has got.
///
/// Only what a person asked for: the refresh button, or the first read of an account just added.
/// A round on the timer, a server saying something arrived and a change on its way to the server
/// are all silent, because nobody is waiting on them.
///
/// Its own command, and deliberately cheap: it reads a number held in memory and asks no database
/// anything. `how_things_are` walks several tables, so it is asked twice a minute — and a round
/// lasts a few seconds, so a line that waited for it would miss every round there was. That is
/// what the progress line did: it showed nothing, because it was never looking when anything was
/// happening.
///
/// Counts and nothing else. Which mailboxes those are is not the window's business and would be a
/// list of somebody's addresses crossing the boundary for a line that shows a number.
#[tauri::command]
pub async fn what_is_being_read(state: State<'_, AppState>) -> Result<BeingReadDto, CommandError> {
    let mailboxes = state.in_flight.how_far();
    let along = state.how_far.along(mailboxes.done, mailboxes.asked);

    Ok(BeingReadDto {
        done: along.mailboxes_done,
        asked: along.mailboxes,
        through: along.through,
        of: along.of,
    })
}

/// How far the reading happening right now has got.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct BeingReadDto {
    /// Mailboxes finished.
    pub done: u32,
    /// Mailboxes this burst of reading was asked for. Nought when nothing is being read.
    pub asked: u32,
    /// How far through the whole burst the reading has got, and out of what.
    ///
    /// The bar moves by these rather than by mailboxes: nine mailboxes read at once start and
    /// finish together, so counted by mailbox a bar is empty for most of a round and then full.
    /// A folder finishes every few hundred milliseconds, all round long, and each mailbox counts
    /// for the same share of the whole however many folders it happens to hold.
    pub through: u32,
    pub of: u32,
}

#[tauri::command]
pub async fn how_things_are(state: State<'_, AppState>) -> Result<HowThingsAreDto, CommandError> {
    let profile = state.storage.profile()?;

    let how = tauri::async_runtime::spawn_blocking(move || {
        ActivityReadModel::new(profile).how_things_are()
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(HowThingsAreDto {
        last_success_at: how.last_success_at,
        stopped_by: how.stopped_by.map(|why| match why {
            crate::ports::activity::WhyItStopped::Offline => WhyItStoppedDto::Offline,
            crate::ports::activity::WhyItStopped::SignIn => WhyItStoppedDto::SignIn,
            crate::ports::activity::WhyItStopped::Storage => WhyItStoppedDto::Storage,
            crate::ports::activity::WhyItStopped::Other => WhyItStoppedDto::Other,
        }),
        accounts_failing: how.accounts_failing,
        waiting: how.waiting,
        sending: how.sending,
        stopped: how.stopped,
        came_back: how.came_back,
    })
}
