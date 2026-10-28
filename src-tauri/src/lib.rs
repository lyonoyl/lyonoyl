// The build made for screenshots reads its mail from the scene it plays, and the code that reaches
// a server is in it all the same, unused: that is the build being what it is, not code to remove.
#![cfg_attr(feature = "screenshots", allow(dead_code))]

#[cfg(test)]
mod account_tests;
mod application;
mod bootstrap;
mod commands;
#[cfg(test)]
mod dev_mail_server;
mod infrastructure;
mod ports;
mod read_model;
#[cfg(test)]
mod rendering_tests;
#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod sending_tests;
#[cfg(test)]
mod transcript_tests;

use application::settings::PendingGeometry;
use bootstrap::Storage;
use infrastructure::html::serving::BodyBeingRead;
use infrastructure::schedule::InFlight;
use infrastructure::secure_storage::cache::CachedCredentials;
use infrastructure::secure_storage::SecretStore;

/// The credential store the application uses: whichever this device has, asked once per account
/// per run.
pub type Credentials = CachedCredentials<SecretStore>;
use tauri::{Manager, RunEvent};

pub struct AppState {
    pub storage: Storage,
    /// Window geometry waiting to be written. Shared by the window events and the exit hook.
    pub geometry: PendingGeometry,
    /// The accounts being read right now. Shared by the scheduler and the refresh command, so one
    /// account is never read twice at the same time.
    pub in_flight: InFlight,
    /// How far the reading happening right now has got, for the line at the foot of the window.
    pub how_far: crate::infrastructure::progress::HowFarAlong,
    /// How many passes a person's window may have underway at once.
    ///
    /// The same width a background round uses, and here for the same reason: a window that asks
    /// about a hundred mailboxes has asked for a hundred connections unless something says
    /// otherwise. Asked for one at a time by the window instead, one slow server would hold up every
    /// account behind it.
    pub reading_at_once: std::sync::Arc<tokio::sync::Semaphore>,
    /// Tells an account's server what a person just did, a moment after they did it. Empty while
    /// there is no storage to read into, and filled once when there is: at the start, or when a
    /// device whose secrets are in a file of its own has that file opened.
    pub pushes: std::sync::OnceLock<crate::infrastructure::pushes::Pushes>,
    /// Files chosen for a message that has not been sent yet. Never crosses to the window.
    pub attaching: crate::infrastructure::attaching::FilesBeingAttached,
    /// The newest state of the message being written, as the window noted it, for the end of the
    /// application to write down. Shared by the commands that note, keep and throw away a draft and
    /// hand a message to the outbox, and by the run callback below.
    pub being_written: crate::application::keep_at_the_end::BeingWritten,
    /// The interface's own words, handed over so a notification can be raised without it.
    pub words: crate::infrastructure::words::InterfaceWords,
    /// Gathers arrivals so a hundred mailboxes finishing at once ring once.
    pub bell: crate::infrastructure::bell::Bell,
    /// Whether the window has the focus of the system, as its events last reported.
    ///
    /// Read at the end of a round, on a background thread, which is why it is a flag here rather
    /// than a question asked of the window: the window can only be asked from the thread that owns
    /// it, and by then that thread is elsewhere.
    pub attention: crate::infrastructure::attention::Focus,
    /// Passwords already read from this device's secret store this run. Shared with the background
    /// rounds, so the store is asked once per account rather than once per pass.
    pub credentials: Credentials,
    /// Where this device keeps its secrets, and whether it can be reached yet. The interface asks,
    /// because a device with no system store keeps the key its databases are written under in a
    /// file, and nothing opens until a master password is given.
    pub secrets: SecretStore,
    /// The body of the message being read, waiting for the frame that will show it. Never handed
    /// across IPC: what the interface gets is the address it is served at.
    pub bodies: BodyBeingRead,
    /// The thread the event loop runs on, taken while standing on it.
    ///
    /// A system dialog is raised there and nowhere else. A command already on that thread would
    /// wait for itself, so the one place that knows the answer records it here.
    pub event_loop: std::thread::ThreadId,
}

/// Every command the window may call. One list: a command that reached only one build
/// would be missing from the other, and nothing compares them.
fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::workspace::get_workspace_overview,
        commands::accounts::default_port_for,
        commands::accounts::test_account_connection,
        commands::accounts::find_server_settings,
        commands::recovery::set_aside_local_storage,
        commands::recovery::restart_after_setting_aside,
        commands::accounts::add_account,
        commands::accounts::change_account,
        commands::updates::look_for_an_update,
        commands::updates::install_the_update,
        commands::updates::what_changed,
        commands::updates::changes_seen,
        commands::updates::restart_into_the_new_version,
        commands::accounts::remove_account,
        commands::accounts::what_is_held,
        commands::settings::show_where_the_journal_writes,
        commands::settings::open_the_support_page,
        commands::settings::system_languages,
        commands::accounts::account_presets,
        commands::accounts::account_hint,
        commands::licences::licences,
        commands::notify::set_notification_words,
        commands::menu::set_menu_words,
        commands::accounts::get_account_settings,
        commands::account_groups::list_account_groups,
        commands::account_groups::create_account_group,
        commands::account_groups::rename_account_group,
        commands::account_groups::remove_account_group,
        commands::account_groups::place_account_in_group,
        commands::account_groups::reorder_accounts,
        commands::account_groups::arrange_sidebar,
        commands::message::opening_message,
        commands::account_groups::set_account_group_collapsed,
        commands::settings::get_preferences,
        commands::settings::set_preference,
        commands::settings::app_icons_wearable,
        commands::settings::get_pane_widths,
        commands::settings::set_pane_width,
        commands::settings::get_sidebar_folded,
        commands::settings::set_sidebar_folded,
        commands::list::get_view_window,
        commands::list::get_conversation,
        commands::mailboxes::get_mailbox_tree,
        commands::mailboxes::set_account_collapsed,
        commands::sync::sync_account,
        commands::message::get_message,
        commands::message::save_message,
        commands::message::save_attachment,
        commands::act::mark_message_read,
        commands::act::move_message,
        commands::act::archive_message,
        commands::act::delete_message,
        commands::act::empty_folder,
        commands::act::try_action_again,
        commands::act::undo_action,
        commands::activity::how_things_are,
        commands::activity::what_is_being_read,
        commands::body::open_message_body,
        commands::body::close_message_body,
        commands::body::open_link,
        commands::body::ask_about_a_written_link,
        commands::secrets::get_secret_store_state,
        commands::secrets::choose_master_password,
        commands::secrets::unlock_secrets,
        commands::compose::send_message,
        commands::compose::attach_file,
        commands::compose::forget_attachment,
        commands::compose::answer_audience,
        commands::compose::forward_message,
        commands::compose::keep_draft,
        commands::compose::note_draft,
        commands::compose::forget_noted_draft,
        commands::compose::kept_drafts,
        commands::compose::sent_here,
        commands::compose::latest_draft,
        commands::compose::reopen_draft,
        commands::compose::discard_draft,
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before anything opens a file. Windows sets a process no limit of this kind.
    #[cfg(unix)]
    infrastructure::open_files::allow_enough();

    let builder = bootstrap::serve_message_bodies(tauri::Builder::default());

    builder
        // First, before anything else is set up: a second copy of this application must stop
        // before it opens a database, not after.
        //
        // Two copies over one profile is not untidiness. `profile.sqlite3` holds the only copy of
        // the accounts, the settings and the queue of what a person asked for; each copy would
        // hold its own watch on every account; and both would take instructions from the same
        // queue and tell the server the same thing twice. The queue defends itself as well — what
        // one round takes, another does not see — and this is the layer that means it never has
        // to.
        //
        // What the second copy does instead is bring the first one forward, which is what somebody
        // opening an application that is already open meant to happen.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                use tauri::Manager;

                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            },
        ))
        .plugin(tauri_plugin_opener::init())
        // Registered so Rust can raise one. The window is granted no permission for it — the
        // capabilities file names the three core permissions the window uses and nothing else —
        // because the window never asks: what is shown is decided in Rust, from a count the
        // window never sees.
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Storage failure must not prevent the window from appearing: the UI shows a recovery
            // screen instead. Only a failure to resolve the data directory is fatal here.
            let app_data_dir = bootstrap::data_directory(app.handle())?;

            // The build made for screenshots starts from nothing every time: its scene is written
            // again, dated from now, into a folder of its own that no other build opens.
            #[cfg(feature = "screenshots")]
            bootstrap::scene::start_over(&app_data_dir)?;

            // Before anything that could want to say something. The folder is beside the databases
            // because that is the folder the owner already knows about, and how much is written is
            // asked for from outside rather than compiled in: `LYONOYL_LOG=detail` for a terminal,
            // and a file called `detail` in the folder itself for somebody who has never opened
            // one — which is also the only way to turn it up on the run where the profile will not
            // open, and that is the run where it is wanted.
            let how_much = how_much_to_write_down(&app_data_dir);
            let _ = infrastructure::journal::install(&app_data_dir, how_much);

            let how_far = crate::infrastructure::progress::HowFarAlong::default();
            let in_flight = InFlight::reporting_to(how_far.clone());
            let reading_at_once = std::sync::Arc::new(tokio::sync::Semaphore::new(
                crate::infrastructure::schedule::AT_ONCE,
            ));
            // Before the databases, because they are opened with the key it holds — and chosen by
            // them, because that key stays in the store it was made in.
            //
            // Gathered into their own folder first: what is beside the profile decides both.
            let (databases, refused) = bootstrap::gather_the_databases(&app_data_dir);
            let secrets = bootstrap::the_store_for(&databases);

            // What the start costs, step by step. Without these the journal's first entry is the
            // first pass over a mailbox, and every second a window spends before that belongs to
            // no record at all.
            let starting = std::time::Instant::now();
            let step = |what: &'static str, since: &mut std::time::Instant| {
                use crate::ports::journal::{Journal, Note, Value};

                let journal = crate::infrastructure::journal::journal();
                if journal.keeps(Note::Detail) {
                    journal.write_down(
                        Note::Detail,
                        what,
                        &[("took", Value::Millis(since.elapsed().as_millis() as u64))],
                    );
                }
                *since = std::time::Instant::now();
            };

            let mut since = starting;
            let (storage, key_on_its_way) = match refused {
                Some(refused) => (refused, None),
                None => bootstrap::open_storage(&databases, &secrets),
            };
            let credentials = CachedCredentials::new(secrets.clone());

            // Mail arrives without anyone asking for it. Nothing runs until the window is up, so a
            // slow server cannot delay the first paint.
            // Made here so the background rounds and the state hold the same one.
            let bell = crate::infrastructure::bell::Bell::default();

            step("start.storage", &mut since);

            // A device whose secrets are in a file opens its databases when that file is opened,
            // and starts what runs on them then. Started now, it would only write down that nothing
            // ever started.
            #[cfg(not(feature = "screenshots"))]
            let pushes = match storage.is_ready() || secrets.master_password().is_none() {
                true => bootstrap::start_background_sync(
                    app.handle(),
                    &storage,
                    in_flight.clone(),
                    credentials.sharing(secrets.clone()),
                    bell.clone(),
                    std::sync::Arc::clone(&reading_at_once),
                ),
                false => None,
            };

            // The build made for screenshots reads its scene once, with the pass every account is
            // read with, and starts nothing that would read again: no round, no watch, no push.
            #[cfg(feature = "screenshots")]
            let pushes: Option<infrastructure::pushes::Pushes> = {
                bootstrap::scene::play(app.handle(), &storage, &secrets)?;
                None
            };

            bootstrap::ring_when_there_is_something_to_say(app.handle(), bell.clone());

            // The settings item in the application's menu, once the window has named it: the
            // window opens them, the way its own keys do.
            app.on_menu_event(|app, event| {
                use tauri::Emitter;

                if event.id() == commands::menu::SETTINGS {
                    let _ = app.emit(commands::menu::OPEN_SETTINGS, ());
                }
            });

            app.manage(AppState {
                storage,
                pushes: pushes.map(std::sync::OnceLock::from).unwrap_or_default(),
                geometry: PendingGeometry::default(),
                attaching: crate::infrastructure::attaching::FilesBeingAttached::default(),
                being_written: crate::application::keep_at_the_end::BeingWritten::default(),
                words: crate::infrastructure::words::InterfaceWords::default(),
                bell,
                attention: crate::infrastructure::attention::Focus::assuming_it_does(),
                in_flight,
                how_far: how_far.clone(),
                reading_at_once,
                credentials,
                secrets,
                bodies: BodyBeingRead::default(),
                // Setup runs on the thread that goes on to run the event loop, so this is the
                // one place in the application where the answer is known rather than assumed.
                event_loop: std::thread::current().id(),
            });

            // A key the store had not handed over when the start stopped waiting: the system may
            // still be asking a person, and the databases open the moment they answer.
            if let Some(coming) = key_on_its_way {
                bootstrap::open_when_the_key_arrives(app.handle().clone(), coming);
            }

            // Whatever was unread when this was last closed. Drawn once here, and after that only
            // when a pass reports: without this the icon is bare until mail moves, so somebody who
            // left unread mail behind opens the application to an icon saying there is none.
            bootstrap::redraw_the_badge(app.handle());

            // Asking the engine what it does, instead of starting. Three of the four rendering layers
            // are properties of a webview this project has never run, and `docs/platforms/differences.md`
            // carries a column of rows marked reasoned rather than measured because of it. This
            // turns them into numbers that mean the same thing on every platform, and it is a
            // thing that can be run again after the engine updates.
            #[cfg(debug_assertions)]
            if std::env::var_os(bootstrap::engine_probe::ASKED_FOR)
                .is_some_and(|asked| !asked.is_empty())
            {
                use tauri::Manager;

                let bodies = app.state::<AppState>().bodies.clone();

                // Opens the frame and returns: the waiting is on a thread of its own, because this
                // runs on the thread that becomes the event loop and waiting here means nothing
                // ever pumps the webview.
                if let Err(why) = bootstrap::engine_probe::ask_the_engine(app.handle(), &bodies) {
                    bootstrap::engine_probe::write_down(&Err(why));
                    std::process::exit(1);
                }
                return Ok(());
            }

            // Built here, after the state it reads from is in place: a window can only be given a
            // navigation handler as it is made, and this one refuses everything except the
            // application's own pages and the scheme a message body is served on. A link inside a
            // message is refused too — which is what keeps the message on screen while the person
            // is asked about it.
            let handle = app.handle().clone();
            bootstrap::window::build(app.handle(), move |name| {
                bootstrap::link_followed(&handle, name);
            })?;

            if let Some(window) = app.get_webview_window("main") {
                step("start.window", &mut since);
                bootstrap::window::restore_and_show(&window);
                bootstrap::window::persist_on_change(&window);
                // The picture chosen in the settings, from the start rather than once the window
                // has asked for anything.
                bootstrap::window::wear_the_chosen_icon(app.handle());
            }
            Ok(())
        })
        .invoke_handler(handler())
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // The only hook that sees every way out the application is told of, on every platform.
            // macOS makes it necessary rather than merely tidy: Cmd+Q terminates the application
            // without ever reaching the window's CloseRequested, and without `ExitRequested`
            // either — only `Exit` comes, and so it does for logging out, there and on Windows. A
            // Linux desktop tells the application nothing as its session ends, so logging out
            // there reaches neither event (`docs/platforms/differences.md`).
            //
            // The message being written goes first: it is the one thing here somebody typed, and
            // once the end has begun, the window that could have kept it cannot be asked anything.
            // Whichever of the two events comes second finds it written and does nothing. Neither
            // is ever held off here: a run that went on after this had written down what was being
            // written would keep nothing of that message at the next way out.
            if is_the_end(&event) {
                keep_what_is_being_written(app);
                bootstrap::window::flush(app);
                infrastructure::journal::flush();
            }
        });
}

/// Whether `event` is the application ending, when what was being written is written down and the
/// rest of this run flushed. `Exit` is the one every way out the application is told of ends in,
/// and on macOS the only one Cmd+Q sends.
fn is_the_end(event: &RunEvent) -> bool {
    matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit)
}

/// Writes down the newest note of the message being written, once, as the application ends.
///
/// On the thread that is ending, because nothing else is left to do it: a write handed to another
/// thread would be dropped when the process exits. The profile is asked for only when there is a
/// note, and a profile that is not open writes nothing.
fn keep_what_is_being_written(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    state.being_written.at_the_end(|| {
        state
            .storage
            .profile()
            .ok()
            .map(infrastructure::sqlite::drafts::SqliteDrafts::new)
    });
}

/// How much of this run is written down.
///
/// Asked for from outside the build, in two ways and in this order. `LYONOYL_LOG` is for a terminal
/// and for the machines the platform pass runs on. A file called `detail` in the log folder
/// is for a person who has never opened a terminal — and for the one run where it matters most,
/// the run where the profile will not open and there is no settings screen to reach.
///
/// Anything unreadable is the quiet answer. Every way of being wrong here has to write less down
/// rather than more, because more is a file holding when somebody read their mail.
fn how_much_to_write_down(beside: &std::path::Path) -> ports::journal::Note {
    if let Ok(asked) = std::env::var("LYONOYL_LOG") {
        return ports::journal::Note::parse(&asked);
    }

    match beside
        .join(infrastructure::journal::files::DIRECTORY)
        .join("detail")
        .exists()
    {
        true => ports::journal::Note::Detail,
        false => ports::journal::Note::Worth,
    }
}

/// That nothing anybody wrote can be talked into writing down a password.
///
/// Twenty-five crates in this graph carry `log`, and the HTTP stack carries `tracing`. Two of those
/// call sites are not noise: `async-imap` writes the request line it is about to send, which is
/// `LOGIN` with the password on it, and the raw response, which is the message. The HTTP client
/// Tauri brings writes the host of every connection it opens; nothing here calls it, and it is
/// compiled in either way. With no receiver none of that reaches anywhere — and a receiver is one
/// line in anybody's `main`, or one `tracing_subscriber::fmt().try_init()` inside a dependency.
///
/// Both crates compile a level check into every call site against a constant their Cargo features
/// set. With the constant off the macro expands to nothing: the arguments are never formatted and
/// no receiver, environment variable or thread-local dispatcher can bring them back. These are
/// what fails if the features are ever dropped from `Cargo.toml`.
#[cfg(test)]
mod tests {
    #[test]
    fn nothing_in_this_binary_can_be_made_to_write_down_what_it_sends() {
        assert_eq!(
            log::STATIC_MAX_LEVEL,
            log::LevelFilter::Off,
            "`log` records are compiled in again -- async-imap writes LOGIN and the password on it"
        );
        assert_eq!(
            tracing::level_filters::STATIC_MAX_LEVEL,
            tracing::level_filters::LevelFilter::OFF,
            "`tracing` records are compiled in again -- the HTTP stack writes the host it dials"
        );
    }
}

/// What ends the application, and with it writes down what was being written.
#[cfg(test)]
mod the_end {
    use super::{is_the_end, RunEvent};

    #[test]
    fn ending_is_exit_and_nothing_short_of_it() {
        // `ExitRequested` carries a handle only tauri can make, so only `Exit` is asked about
        // here: it follows `ExitRequested` whenever nothing holds the end off, and nothing in this
        // application does.
        assert!(is_the_end(&RunEvent::Exit));
        assert!(!is_the_end(&RunEvent::Ready));
        assert!(!is_the_end(&RunEvent::Resumed));
    }
}
