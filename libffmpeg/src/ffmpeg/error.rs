use libcmd::CommandError;
use libwhich::WhichError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use valuable::Valuable;

#[derive(Debug, Clone, Serialize, Deserialize, Valuable, Error)]
pub enum FfmpegError {
    #[error(transparent)]
    Command {
        #[from]
        inner_error: CommandError,
    },
    #[error(transparent)]
    Which {
        #[from]
        inner_error: WhichError,
    },
    #[error(
        "Unable to locate ffmpeg on your PATH, set LIBFFMPEG_FFMPEG_PATH to the binary, or update your PATH"
    )]
    NotFound,
}
