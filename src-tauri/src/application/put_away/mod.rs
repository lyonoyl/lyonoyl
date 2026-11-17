//! Putting away the message a notification named, from the notification.
//!
//! What a notice's button asks for is what the window's own buttons ask for: the trash is a delete
//! asked for what can be got back, the junk a move into the account's junk folder. So the server is
//! told the same way, a server that cannot move is handled the same way, and the way back is the one
//! the window's key already takes.
//!
//! What differs is when. A button in the window is pressed on a message the window has just shown;
//! a notice's is pressed whenever somebody gets to it, and a notice stays in the system's list long
//! after what it said stopped being true. So the message is looked for where the notice saw it, and
//! the place it is to go is looked for in its account, before anything is asked for.

use crate::application::act_on_mail::ActOnMail;
use crate::application::error::UseCaseError;
use crate::ports::notifier::PutInto;
use crate::ports::op_log::OperationLog;
use crate::ports::placement::Placements;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::operation::{DeleteIntent, OperationKind, OperationTarget};

/// What became of a press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutAwayOutcome {
    /// Asked for into the trash, as the window's trash button asks.
    Deleted,
    /// Asked for into the account's junk folder.
    Moved,
    /// Nothing asked for: the message is no longer where the notice saw it, or a delete is taking it
    /// away. Filed or deleted since — in the window, by an earlier press, or on another device — and
    /// an instruction about it now would stop for want of the message, on a row of a folder it is no
    /// longer in.
    NotThere,
    /// Nothing asked for: the account keeps no such folder now. Without a trash the delete would
    /// stop with nowhere to keep the message, and without a junk folder there is nowhere to move it.
    NowhereToPut,
}

pub struct PutAway<L: OperationLog, P: Placements> {
    operations: L,
    placements: P,
}

impl<L: OperationLog, P: Placements> PutAway<L, P> {
    pub fn new(operations: L, placements: P) -> Self {
        Self {
            operations,
            placements,
        }
    }

    /// `number` is what the index knows the account by, which is how its trash and its junk folder
    /// are found.
    pub fn execute(
        &self,
        account_id: &str,
        number: i64,
        message: OperationTarget,
        into: PutInto,
    ) -> Result<PutAwayOutcome, UseCaseError> {
        if self
            .placements
            .where_it_sits(message.folder, message.message)?
            .is_none()
        {
            return Ok(PutAwayOutcome::NotThere);
        }

        // On its way out already, by a delete the server has yet to be told of: nothing asked for
        // after a delete folds into it, so a move would stop for want of the message, and a second
        // delete adds nothing to the first.
        let deleting = self
            .operations
            .outstanding_for(account_id, message)?
            .iter()
            .any(|waiting| matches!(waiting.operation.kind, OperationKind::Delete { .. }));
        if deleting {
            return Ok(PutAwayOutcome::NotThere);
        }

        let role = match into {
            PutInto::Trash => FolderRole::Trash,
            PutInto::Junk => FolderRole::Junk,
        };
        let Some(place) = self.placements.folder_for(number, role)? else {
            return Ok(PutAwayOutcome::NowhereToPut);
        };

        let acting = ActOnMail::new(&self.operations);

        match into {
            // A delete, not a move into the folder just found: where a delete goes is decided when
            // the server is told, against the account as it is then, as for the window's button.
            PutInto::Trash => acting
                .delete(account_id, message, DeleteIntent::ToTrash)
                .map(|()| PutAwayOutcome::Deleted),
            PutInto::Junk => acting
                .move_to(account_id, message, place)
                .map(|()| PutAwayOutcome::Moved),
        }
    }
}

#[cfg(test)]
mod tests;
