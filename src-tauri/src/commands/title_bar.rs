//! The strip at the top of the window, standing in for the title bar macOS no longer draws there:
//! the window says what landed on it, and the core does what the bar would have done.

use tauri::Window;

use super::error::{CommandError, ErrorCode};
use crate::infrastructure::title_bar;

/// The strip was pressed: the window follows the pointer until the button comes up.
#[tauri::command]
pub async fn press_title_bar(window: Window) -> Result<(), CommandError> {
    title_bar::pressed(&window).map_err(|_| unanswered())
}

/// The strip was double-clicked: the window does what a double click on a title bar does.
#[tauri::command]
pub async fn double_click_title_bar(window: Window) -> Result<(), CommandError> {
    title_bar::double_clicked(&window).map_err(|_| unanswered())
}

fn unanswered() -> CommandError {
    CommandError::new(
        ErrorCode::Internal,
        "The window did not answer its title bar.",
    )
}
