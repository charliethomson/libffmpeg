use std::path::PathBuf;

use crate::tools::{Tool, find};

/// Environment override for ffmpeg's location.
///
/// Prefer [`crate::tools::set_tool_path`] in new code: it needs no `unsafe` and
/// works after threads have started.
pub const FFMPEG_PATH_OVERRIDE_KEY: &str = Tool::Ffmpeg.env_key();

/// Resolve the ffmpeg binary. See [`crate::tools::locate`] to also learn where
/// it was found.
#[must_use]
pub fn find_ffmpeg() -> Option<PathBuf> {
    find(Tool::Ffmpeg)
}
