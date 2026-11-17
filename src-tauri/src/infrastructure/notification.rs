use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::ports::notifier::{Notice, Notifier, NotifyError, PutInto};
use lyonoyl_domain::operation::OperationTarget;

/// What is told which button somebody pressed on a notice, and the message the notice named.
pub type Answered = Arc<dyn Fn(OperationTarget, PutInto) + Send + Sync>;

/// Hands a notice to the operating system.
///
/// It is this small because the port is: two strings, already decided and already cleaned, and
/// the buttons decided beside them. Nothing here chooses what a notice says, and nothing here can
/// add to it — a subject reaches the system only because somebody asked for one in the settings,
/// and the place that read that setting is `AnnounceMail`.
///
/// **Two ways to the screen, and they differ.** A built copy on macOS talks to the system's
/// notification centre itself, which is what draws buttons and says which one was pressed. Every
/// other run goes through the notification plugin, which draws the two strings and nothing else:
/// Windows and Linux, a run outside a bundle — `npm run app`, a bare binary out of `target`, which
/// the centre does not know and stops the process for — and a copy the centre will not take, which
/// an unsigned build may be. Those show no buttons.
///
/// **The icon and the name beside it are the bundle's, not this code's.** macOS attributes a
/// notification to the application bundle that posted it, so a run with no bundle is attributed
/// to whatever launched it, and the terminal's icon appears instead of this application's. Nothing
/// here can set it; a built application carries `CFBundleIdentifier` and `icon.icns` and is
/// attributed to itself.
///
/// On macOS the plugin's library installs a hook that makes the main bundle report a different
/// identifier. In a build the system knows, the identifier it substitutes is this application's
/// own, so both states answer the same string; the one bad state needs a build the system has
/// never seen, and that cannot come before the first keychain read because a notification needs a
/// sync and a sync needs the database key. Written down because it is the kind of thing somebody
/// would otherwise discover by finding the keychain asking for permission.
pub struct SystemNotifier {
    app: AppHandle,
    // Read on macOS alone: the plugin the other two systems show through hears nothing back.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    answered: Answered,
}

impl SystemNotifier {
    pub fn new(app: AppHandle, answered: Answered) -> Self {
        Self { app, answered }
    }
}

impl Notifier for SystemNotifier {
    fn show(&self, notice: &Notice) -> Result<(), NotifyError> {
        #[cfg(target_os = "macos")]
        if centre::answers_here() {
            centre::show(self.app.clone(), notice.clone(), Arc::clone(&self.answered));
            return Ok(());
        }

        in_words_alone(&self.app, notice)
    }
}

/// The two strings, through the plugin, and no buttons.
fn in_words_alone(app: &AppHandle, notice: &Notice) -> Result<(), NotifyError> {
    app.notification()
        .builder()
        .title(&notice.title)
        .body(&notice.body)
        .show()
        .map_err(|_| NotifyError::Refused)
}

/// Takes away the notices with buttons this run showed, as it ends.
///
/// A notice's buttons are answered by the run that showed it, and nothing else can answer them:
/// left behind, a button pressed would do nothing, and the system starts an application to hear one
/// of its notices with buttons dismissed — so one cleared from the screen after the application has
/// quit would open it again. A notice without buttons stays, saying what it said. Nothing on the
/// other two systems, whose notices carry no buttons.
pub fn take_away_what_was_shown() {
    #[cfg(target_os = "macos")]
    if centre::answers_here() {
        centre::take_away_what_was_shown();
    }
}

/// Takes away the notices with buttons a run that did not end cleanly left behind, for the same
/// reason: nothing in this run can answer them.
pub fn clear_what_an_earlier_run_left() {
    #[cfg(target_os = "macos")]
    if centre::answers_here() {
        centre::clear_what_an_earlier_run_left();
    }
}

/// The system's notification centre, which this application talks to in a built copy on macOS.
#[cfg(target_os = "macos")]
mod centre {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use mac_usernotifications::{Action, Notification};
    use tauri::AppHandle;

    use super::Answered;
    use crate::ports::notifier::{Notice, Offers, PutInto};

    /// What the centre hands back for each button. Its own names, never words a person reads.
    const INTO_TRASH: &str = "lyonoyl.into-trash";
    const INTO_JUNK: &str = "lyonoyl.into-junk";

    /// How the name of every notice with buttons begins: what a later run knows its leftovers by.
    const WITH_BUTTONS: &str = "lyonoyl.with-buttons.";

    /// The longest the end of a run waits for its notices to be taken away. The centre is another
    /// process, and an application that will not quit because of it is worse than a notice left.
    const TAKING_AWAY_AT_MOST: Duration = Duration::from_millis(500);

    /// How often the end of a run looks whether they have gone, and how many times at most.
    const LOOKING_AGAIN: Duration = Duration::from_millis(25);
    const LOOKS: u32 = 20;

    /// The notices with buttons this run showed, by the names the centre knows them by.
    static SHOWN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// How many notices this run has asked to show, which numbers each.
    static ASKED: AtomicU64 = AtomicU64::new(0);

    /// Whether this run shows through the centre, decided once.
    ///
    /// Only from a bundle the system can name: the centre ends the process of anything else rather
    /// than answering it, and a development run (`tauri::is_dev`) is a bare binary whose notices the
    /// plugin attributes to the terminal by design.
    pub(super) fn answers_here() -> bool {
        static HERE: OnceLock<bool> = OnceLock::new();

        *HERE.get_or_init(|| {
            !tauri::is_dev()
                && from_a_bundle(std::env::current_exe().ok().as_deref())
                && mac_usernotifications::check_bundle().is_ok()
        })
    }

    /// Whether the program is the one inside an application's bundle, `<name>.app/Contents/MacOS/`.
    pub(super) fn from_a_bundle(program: Option<&std::path::Path>) -> bool {
        let Some(directory) = program.and_then(std::path::Path::parent) else {
            return false;
        };
        let contents = directory.parent();
        let bundle = contents.and_then(std::path::Path::parent);

        directory.file_name().is_some_and(|name| name == "MacOS")
            && contents
                .and_then(std::path::Path::file_name)
                .is_some_and(|name| name == "Contents")
            && bundle
                .and_then(std::path::Path::extension)
                .is_some_and(|extension| extension == "app")
    }

    /// The name the centre gives back for a button.
    pub(super) fn named(into: PutInto) -> &'static str {
        match into {
            PutInto::Trash => INTO_TRASH,
            PutInto::Junk => INTO_JUNK,
        }
    }

    /// Which button a name the centre gave back is, or nothing: a press on the notice itself, a
    /// dismissal, or a name this build never gave.
    pub(super) fn pressed(name: &str) -> Option<PutInto> {
        match name {
            INTO_TRASH => Some(PutInto::Trash),
            INTO_JUNK => Some(PutInto::Junk),
            _ => None,
        }
    }

    /// Which of a notice's own buttons a press was, or nothing: a button it did not carry is not
    /// one somebody could have pressed on it.
    pub(super) fn answer(offers: &Offers, name: &str) -> Option<PutInto> {
        pressed(name).filter(|into| offers.each.iter().any(|offer| offer.into == *into))
    }

    /// The buttons, each asking for the Mac to be unlocked before it does anything: a notice is
    /// shown on a locked screen, and whoever is passing it must not be able to throw somebody's
    /// mail away.
    pub(super) fn buttons(offers: &Offers) -> Vec<Action> {
        offers
            .each
            .iter()
            .map(|offer| Action::button(named(offer.into), &offer.said).requires_authentication())
            .collect()
    }

    /// Shows the notice, and waits for a press for as long as this run lasts.
    ///
    /// On the runtime rather than the thread that asked: what the centre answers is heard on the
    /// main thread's run loop, which the application turns, and handed to this task. A notice nobody
    /// touches keeps its task waiting until the end of the run, as one cleared with the rest of the
    /// centre's list does, since the centre says nothing about that; each costs a few bytes.
    pub(super) fn show(app: AppHandle, notice: Notice, answered: Answered) {
        let number = numbered();

        tauri::async_runtime::spawn(async move {
            let allowed = allowed().await;

            // The first time, the wait is the person's answer to the system's question, and every
            // notice asked for meanwhile waited for it too: only the newest still says what is
            // true, and the older ones would all appear at once, about mail since read or put away.
            if !still_the_newest(number) {
                return;
            }

            // A copy the centre will not take, which an unsigned build may be, is told so here and
            // shows the way every other run does rather than not at all. One the person turned off
            // shows nothing either way.
            if !allowed {
                let _ = super::in_words_alone(&app, &notice);
                return;
            }

            let mut asked = Notification::new()
                .title(&notice.title)
                .message(&notice.body);

            // A notice with no buttons has nothing to hear and nothing to take away: a press on it
            // brings the application forward, which the system does by itself, and dismissing it
            // starts nothing.
            let Some(offers) = notice.offers else {
                let _ = asked.send().await;
                return;
            };

            for button in buttons(&offers) {
                asked = asked.action(button);
            }
            let name = format!("{WITH_BUTTONS}{}-{number}", std::process::id());

            let Ok(sent) = asked.id(&name).send().await else {
                return;
            };

            if let Ok(mut shown) = SHOWN.lock() {
                shown.push(name);
            }

            let Ok(response) = sent.response().await else {
                return;
            };

            if let Some(into) = answer(&offers, &response.action_identifier) {
                answered(offers.about, into);
            }
        });
    }

    /// The next notice's number, the newest the highest.
    fn numbered() -> u64 {
        ASKED.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Whether no notice has been asked for since the one numbered so.
    fn still_the_newest(number: u64) -> bool {
        ASKED.load(Ordering::SeqCst) == number
    }

    /// Whether the centre lets this application's notices through, asked once a run: the first
    /// time, the system asks the person.
    async fn allowed() -> bool {
        static ALLOWED: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();

        *ALLOWED
            .get_or_init(|| async { mac_usernotifications::request_auth().await.unwrap_or(false) })
            .await
    }

    pub(super) fn take_away_what_was_shown() {
        let shown = SHOWN
            .lock()
            .map(|mut held| std::mem::take(&mut *held))
            .unwrap_or_default();
        if shown.is_empty() {
            return;
        }

        // Waited for, because the process ends right after; but only so long.
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // The newest first: it is the likeliest to be on the screen still.
            for name in shown.iter().rev() {
                mac_usernotifications::block_on(mac_usernotifications::close_delivered(name));
            }

            // The centre takes them away on a thread of its own after it is asked, so the end of
            // the run looks until none of them is listed.
            for _ in 0..LOOKS {
                let left = mac_usernotifications::block_on(
                    mac_usernotifications::get_delivered_notification_ids(),
                );
                if !left.iter().any(|name| shown.contains(name)) {
                    break;
                }
                std::thread::sleep(LOOKING_AGAIN);
            }

            let _ = done.send(());
        });
        let _ = finished.recv_timeout(TAKING_AWAY_AT_MOST);
    }

    pub(super) fn clear_what_an_earlier_run_left() {
        tauri::async_runtime::spawn(async {
            for name in mac_usernotifications::get_delivered_notification_ids().await {
                // Only a notice with buttons, and never one this run has shown already, however the
                // two crossed.
                let ours = SHOWN.lock().is_ok_and(|shown| shown.contains(&name));
                if name.starts_with(WITH_BUTTONS) && !ours {
                    mac_usernotifications::close_delivered(&name).await;
                }
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use lyonoyl_domain::message::{FolderId, MessageId};
        use lyonoyl_domain::operation::OperationTarget;

        use crate::ports::notifier::Offer;

        #[test]
        fn a_button_pressed_is_the_button_it_was_given_as() {
            for into in [PutInto::Trash, PutInto::Junk] {
                assert_eq!(pressed(named(into)), Some(into));
            }
            assert_ne!(named(PutInto::Trash), named(PutInto::Junk));
        }

        #[test]
        fn a_press_that_is_not_a_button_puts_nothing_away() {
            // The centre's own names for a press on the notice and for a dismissal, and names close
            // to the buttons' own: none of them may throw a message away.
            for name in [
                "com.apple.UNNotificationDefaultActionIdentifier",
                "com.apple.UNNotificationDismissActionIdentifier",
                "",
                "lyonoyl.into-trash ",
                "LYONOYL.INTO-TRASH",
                "into-trash",
                "Move to trash",
            ] {
                assert_eq!(pressed(name), None, "{name:?}");
            }
        }

        #[test]
        fn a_press_is_taken_only_for_a_button_the_notice_carried() {
            let only_junk = Offers {
                about: OperationTarget::new(
                    MessageId::new(7).expect("an identifier"),
                    FolderId::new(1).expect("a folder"),
                ),
                each: vec![Offer {
                    into: PutInto::Junk,
                    said: "Move to junk".to_owned(),
                }],
            };

            assert_eq!(
                answer(&only_junk, named(PutInto::Junk)),
                Some(PutInto::Junk)
            );
            assert_eq!(answer(&only_junk, named(PutInto::Trash)), None);
            assert_eq!(
                answer(
                    &only_junk,
                    "com.apple.UNNotificationDefaultActionIdentifier"
                ),
                None
            );
        }

        #[test]
        fn a_notice_asked_for_after_another_is_the_only_one_still_the_newest() {
            let older = numbered();
            let newer = numbered();

            assert!(!still_the_newest(older), "the older one waits for nothing");
            assert!(still_the_newest(newer));
        }

        #[test]
        fn every_button_asks_for_the_mac_to_be_unlocked() {
            let offers = Offers {
                about: OperationTarget::new(
                    MessageId::new(7).expect("an identifier"),
                    FolderId::new(1).expect("a folder"),
                ),
                each: vec![
                    Offer {
                        into: PutInto::Trash,
                        said: "Move to trash".to_owned(),
                    },
                    Offer {
                        into: PutInto::Junk,
                        said: "Move to junk".to_owned(),
                    },
                ],
            };

            let made = buttons(&offers);

            assert_eq!(made.len(), 2);
            for (button, offer) in made.iter().zip(&offers.each) {
                match button {
                    Action::Button {
                        identifier,
                        title,
                        requires_authentication,
                    } => {
                        assert_eq!(identifier, named(offer.into));
                        assert_eq!(title, &offer.said, "the window's words, as handed over");
                        assert!(*requires_authentication, "{title} works on a locked Mac");
                    }
                    Action::Reply { .. } => panic!("a notice about mail takes no reply"),
                }
            }
        }

        #[test]
        fn only_the_program_inside_a_bundle_is_from_one() {
            use std::path::Path;

            for (program, from_one) in [
                (
                    "/Applications/Lyonoyl Mail.app/Contents/MacOS/lyonoyl",
                    true,
                ),
                ("/a/b/Lyonoyl Mail.app/Contents/MacOS/lyonoyl", true),
                ("/work/target/debug/lyonoyl", false),
                ("/work/target/release/lyonoyl", false),
                ("/a/Lyonoyl Mail.app/Contents/Resources/lyonoyl", false),
                ("/a/Lyonoyl Mail/Contents/MacOS/lyonoyl", false),
                ("/a/Lyonoyl Mail.bundle/Contents/MacOS/lyonoyl", false),
                ("/a/Lyonoyl Mail.app/MacOS/lyonoyl", false),
                ("lyonoyl", false),
            ] {
                assert_eq!(
                    from_a_bundle(Some(Path::new(program))),
                    from_one,
                    "{program}"
                );
            }
            assert!(!from_a_bundle(None));
        }

        #[test]
        fn the_test_binary_is_not_from_a_bundle() {
            // What keeps the centre from being asked by `cargo test`, which it would end rather than
            // answer. Asked of the program itself as well, since a test run is a development run
            // and that alone already says no.
            assert!(!from_a_bundle(std::env::current_exe().ok().as_deref()));
            assert!(!answers_here());
        }
    }
}
