use std::path::PathBuf;

use crate::util::find_binary;

pub const FFMPEG_PATH_OVERRIDE_KEY: &str = "LIBFFMPEG_FFMPEG_PATH";

pub fn find_ffmpeg() -> Option<PathBuf> {
    find_binary("ffmpeg", FFMPEG_PATH_OVERRIDE_KEY)
}
