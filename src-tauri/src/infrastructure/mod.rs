pub mod app_icon;
pub mod attaching;
pub mod attention;
pub mod badge;
pub mod bell;
pub mod dialog;
#[cfg(any(test, debug_assertions))]
/// Fixtures only, now that nothing shipped invents mail: a development account is filled by its
/// server like anybody else's. Compiled out of the application entirely.
#[cfg(test)]
pub mod fake_provider;
pub mod html;
pub mod http;
pub mod imap;
pub mod journal;
pub mod net;
pub mod notification;
/// Unix only: Windows sets a process no limit of this kind.
#[cfg(unix)]
pub mod open_files;
pub mod progress;
pub mod pushes;
/// A made-up mailbox for the build made for screenshots, and for the tests of it. Compiled out of
/// every other build.
#[cfg(any(test, feature = "screenshots"))]
pub mod scene;
pub mod schedule;
pub mod secure_storage;
pub mod smtp;
pub mod sqlite;
pub mod updates;
pub mod words;
