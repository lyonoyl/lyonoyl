//! The updater, put together: the places the project publishes to, and where this application keeps
//! its files.

use std::sync::Arc;

use crate::infrastructure::net::TransportPolicy;
use crate::infrastructure::updates::asking_the_project::AskingTheProject;
use crate::infrastructure::updates::providers::the_projects_providers;

/// The updater, built when it is asked for rather than held: it is reached twice a day at most, and
/// holding a client open for that is a connection nobody is using.
///
/// Answers why it could not be built, in words for a report.
pub fn the_updater(app: &tauri::AppHandle) -> Result<AskingTheProject, &'static str> {
    // The one place that answers where this application keeps its files. Worked out here once and
    // handed down, rather than recomputed by whoever needs it -- which is how the updater came to
    // keep its state somewhere the databases were not.
    let beside = super::data_directory(app).map_err(|_| "There is nowhere to keep files.")?;

    let road = Arc::new(TransportPolicy::new().map_err(|_| "The trust policy is unavailable.")?);

    Ok(AskingTheProject::new(
        the_projects_providers(road, tauri::async_runtime::handle().inner().clone()),
        beside,
    ))
}
