use std::path::PathBuf;

use crate::tools::{Tool, find};

/// Environment override for ffprobe's location.
///
/// Prefer [`crate::tools::set_tool_path`] in new code: it needs no `unsafe` and
/// works after threads have started.
pub const FFPROBE_PATH_OVERRIDE_KEY: &str = Tool::Ffprobe.env_key();

/// Resolve the ffprobe binary. See [`crate::tools::locate`] to also learn where
/// it was found.
#[must_use]
pub fn find_ffprobe() -> Option<PathBuf> {
    find(Tool::Ffprobe)
}
