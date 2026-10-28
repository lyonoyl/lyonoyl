//! What a person does in the build made for screenshots: taken, and kept nowhere.
//!
//! Marking a message, putting it away, throwing it out and sending one are each answered the way a
//! server answers something it has done, so the window goes on as it would. None of it reaches
//! anything: no connection is opened, the scene is not changed, and the next start plays it again
//! from the beginning.

use crate::ports::mail_actions::{
    Acting, Filing, FilingError, MailActionError, MailActions, RefusedSearch, SentMail,
    ServerAction,
};
use crate::ports::mail_submission::{MailSubmission, SubmissionError};
use lyonoyl_domain::outgoing::Outgoing;

/// Says yes to everything, and does none of it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Agrees;

impl MailActions for Agrees {
    fn reach(&self) -> Result<Box<dyn Acting + '_>, MailActionError> {
        Ok(Box::new(Agrees))
    }
}

impl Acting for Agrees {
    fn carry_out(&mut self, _action: ServerAction<'_>) -> Result<(), MailActionError> {
        Ok(())
    }
}

impl SentMail for Agrees {
    fn file(
        &self,
        _message_id: &str,
        _message: &Outgoing,
        _written_at: i64,
        _refused_search: RefusedSearch,
    ) -> Result<Filing, FilingError> {
        Ok(Filing::Added)
    }
}

impl MailSubmission for Agrees {
    fn submit(
        &self,
        _message_id: &str,
        _message: &Outgoing,
        _written_at: i64,
    ) -> Result<(), SubmissionError> {
        Ok(())
    }
}
