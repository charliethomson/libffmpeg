use libcmd::CommandError;
use libwhich::WhichError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use valuable::Valuable;

/// Errors that can occur when running ffmpeg.
#[derive(Debug, Clone, Serialize, Deserialize, Valuable, Error)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "$type",
    content = "context"
)]
pub enum FfmpegError {
    #[serde(rename = "dev.thmsn.ffmpeg.run.error.command")]
    #[error(transparent)]
    Command {
        #[from]
        inner_error: CommandError,
    },
    #[serde(rename = "dev.thmsn.ffmpeg.run.error.which")]
    #[error(transparent)]
    Which {
        #[from]
        inner_error: WhichError,
    },
    #[serde(rename = "dev.thmsn.ffmpeg.run.error.not_found")]
    #[error(
        "Unable to locate ffmpeg on your PATH, set LIBFFMPEG_FFMPEG_PATH to the binary, or update your PATH"
    )]
    NotFound,
}
