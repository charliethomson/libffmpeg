use libcmd::CommandError;
use libwhich::WhichError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use valuable::Valuable;

/// Errors that can occur when running ffprobe.
#[derive(Debug, Clone, Serialize, Deserialize, Valuable, Error)]
pub enum FfprobeError {
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
        "Unable to locate ffprobe on your PATH, set LIBFFMPEG_FFPROBE_PATH to the binary, or update your PATH"
    )]
    NotFound,
}
