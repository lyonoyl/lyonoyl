use tauri::State;

use serde::Serialize;
use ts_rs::TS;

use crate::application::error::UseCaseError;
use crate::application::sync::SyncAccount;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::read_model::mailboxes::MailboxReadModel;
use crate::AppState;

/// What one pass over one account brought in.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct SyncReportDto {
    pub folders: u32,
    /// Messages stored that were not there before, whether they are new or older than what was
    /// already read.
    pub arrived: u32,
    /// Folders that still have history below what has been read.
    pub folders_incomplete: u32,
    /// Folders whose pass failed. The rest of the account still synchronised.
    pub folders_failed: u32,
}

/// Reads one account: which mailboxes it has, then what is in each of them.
///
/// Reading only. Nothing is created, renamed, moved or removed on the server; no folder and no
/// message is removed here either.
#[tauri::command]
pub async fn sync_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<SyncReportDto, CommandError> {
    let told_about = account_id.clone();
    let profile = state.storage.profile()?;
    // The account first, then its file. A pass belongs to one account and reads nothing of another,
    // which is what makes it able to name the file before opening it.
    let number = crate::read_model::mailboxes::index_id_of(&profile, &account_id)
        .map_err(CommandError::from)?
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "That account is not on this device.",
            )
        })?;
    let index = state.storage.index_of_account(number)?;
    let runtime = tauri::async_runtime::handle().inner().clone();

    // Held for the whole pass, so a round coming round does not open a second connection to the
    // same server.
    //
    // Background work holding the account is waited for; another read a person asked for is
    // said so, rather than answered with a report of nothing. Those two look the same to a caller
    // — a pass that found no folders and a pass that never ran — and the caller acts on it: the
    // sidebar records the account as asked and does not ask again for the rest of the session, so
    // a refusal answered as nothing would leave an account empty until the application restarts.
    let asked_at = std::time::Instant::now();

    // Waits its turn where the window has asked about more mailboxes than may talk to a server at
    // once. Held for the whole pass, and the same width a background round uses: a window that
    // asks about a hundred accounts has asked for a hundred connections unless something here
    // says otherwise. Waiting is not refusing — every account asked for is read, in turn.
    let _turn = state
        .reading_at_once
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| CommandError::new(ErrorCode::Internal, "Reading was shut down."))?;

    // A round, a watcher or a push holding the account is waited for rather than answered with
    // "being read": all three are silent, and a refresh turned away by work nobody can see is a
    // button that does nothing. Only a read a person already asked for answers for this one.
    let Some(claim) = state
        .in_flight
        .claim_waiting(&account_id, crate::infrastructure::schedule::Why::Person)
        .await
    else {
        return Err(CommandError::new(
            ErrorCode::AlreadyReading,
            "That account is being read right now.",
        ));
    };

    // The same held passwords the background rounds use: the store is asked once per
    // account per run, not once per pass.
    let state_credentials = state.credentials.sharing(state.secrets.clone());

    let how_far = state.how_far.clone();

    let pass = tauri::async_runtime::spawn_blocking(move || {
        let _claim = claim;
        let written_to = profile.clone();

        let number = MailboxReadModel::new(profile.clone(), index.clone())
            .index_id_of(&account_id)?
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::InvalidRequest,
                    "That account is not on this device.",
                )
            })?;

        // Its end written down the way a round writes down its own, so the foot of the window counts
        // the account as current, or as not updating, whichever kind of pass read it last.
        let sealing = match adapters::message_key(&state_credentials) {
            Ok(sealing) => sealing,
            Err(_) => {
                crate::bootstrap::workspace_sync::a_pass_ended(
                    &written_to,
                    &account_id,
                    Err(&UseCaseError::StorageUnavailable),
                );
                return Err(CommandError::new(
                    ErrorCode::Internal,
                    "The mail key is unavailable.",
                ));
            }
        };

        let pass = SyncAccount::new(
            adapters::accounts(profile.clone()),
            &state_credentials,
            crate::bootstrap::mail_servers(runtime),
            adapters::mail_index(index.clone()),
            adapters::operation_log(profile),
            adapters::message_bodies(index, sealing),
            &how_far,
        )
        .execute(&account_id, number);
        crate::bootstrap::workspace_sync::a_pass_ended(
            &written_to,
            &account_id,
            pass.as_ref().map(|_| ()),
        );
        pass.map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    // What it cost, written down the same way a round writes it. Without this a pass somebody
    // asked for — the refresh button, or the sidebar asking about an account with no folders —
    // left no trace at all, and the journal answered "why is this slow" about only some of them.
    crate::infrastructure::journal::wrote_down_a_pass(&pass, asked_at.elapsed());

    // Said the same way a background round says it. Without this, a pass somebody asked for
    // changed the mail and only the pane that asked knew: the account's folders appeared in the
    // sidebar and the list beside it went on showing what it had — including nothing at all,
    // which is what an account looks like for the minute after it is added.
    // What the pass let go -- instructions that were waiting for exactly this read -- goes to the
    // server now, the way a round sends it on after its own pass. Left for the next round, a way
    // back already shown as done waits minutes for what a push tells the server in a second.
    if pass.operations_found > 0 {
        if let Some(pushes) = state.pushes.get() {
            pushes.please_push(&told_about);
        }
    }

    // Through the function a round says it with, so the window reads again and the number on the
    // icon is redrawn with it: told only to the window, the list showed the new mail and the icon
    // kept the old count.
    if pass.arrived > 0 || pass.updated > 0 || pass.removed > 0 {
        crate::bootstrap::mail_changed(&app, &told_about, pass.arrived, pass.updated, pass.removed);
    }

    Ok(SyncReportDto {
        folders: pass.folders,
        arrived: pass.arrived,
        folders_failed: pass.folders_failed,
        folders_incomplete: pass.folders_incomplete,
    })
}
