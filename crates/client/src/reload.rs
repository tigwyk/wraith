//! Config hot reload, diagnostics toasts and the frame-time overlay (Task 13).
use std::path::Path;

use wraith_config::{Action, Diagnostic};

use crate::app::App;

pub(crate) fn start(_app: &mut App, _path: &Path, _diags: Vec<Diagnostic>) {}
pub(crate) fn action(_app: &mut App, _a: Action) {}
pub(crate) fn note_input(_app: &mut App) {}
pub(crate) fn poll(_app: &mut App) {}
pub(crate) fn presented(_app: &mut App) {}
