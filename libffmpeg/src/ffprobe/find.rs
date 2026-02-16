use std::path::PathBuf;

use crate::util::find_binary;

pub const FFPROBE_PATH_OVERRIDE_KEY: &str = "LIBFFMPEG_FFPROBE_PATH";

pub fn find_ffprobe() -> Option<PathBuf> {
    find_binary("ffprobe", FFPROBE_PATH_OVERRIDE_KEY)
}
