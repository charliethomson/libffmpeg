use std::{path::Path, time::Duration};

use liberror::AnyError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use valuable::Valuable;

use crate::{
    env::find::{FindBinaryError, find_binary_env},
    util::{
        cmd::{self, CommandError, CommandExit},
        exit::CommandExitCode,
    },
};

#[derive(Error, Debug, Clone, Serialize, Deserialize, Valuable)]
pub enum DurationError {
    #[error(transparent)]
    Command {
        #[from]
        inner_error: CommandError,
    },
    #[error(transparent)]
    FindBinary {
        #[from]
        inner_error: FindBinaryError,
    },

    #[error("Process returned, but no exit status was present")]
    IncompleteSubprocess { result: CommandExit },
    #[error("ffprobe exited unsucessfully")]
    ExitedUnsucessfully { exit_code: CommandExitCode },
    #[error("Expected ffprobe to output a line with the duration")]
    ExpectedLine { result: CommandExit },
    #[error("Failed to parse duration provided by ffprobe: {inner_error}")]
    Parse { inner_error: AnyError },
    #[error(
        "Unable to locate ffprobe on your PATH, set LIBFFMPEG_FFPROBE_PATH to the binary, or update your PATH"
    )]
    FfprobeNotFound,
}

pub async fn get_duration<P: AsRef<Path>>(
    input: P,
    cancellation_token: CancellationToken,
) -> Result<Duration, DurationError> {
    let Some(ffprobe_path) = find_binary_env("ffprobe").await? else {
        return Err(DurationError::FfprobeNotFound);
    };

    let mut result = cmd::run(ffprobe_path, None, cancellation_token, move |cmd| {
        cmd.arg("-threads").arg("4");
        cmd.arg("-v").arg("quiet");
        cmd.arg("-show_entries").arg("format=duration");
        cmd.arg("-of").arg("default=noprint_wrappers=1:nokey=1");
        cmd.arg(input.as_ref());
    })
    .await?;

    let Some(exit_code) = result.exit_code.take() else {
        return Err(DurationError::IncompleteSubprocess { result });
    };

    if !exit_code.success {
        return Err(DurationError::ExitedUnsucessfully { exit_code });
    }

    let Some(duration_line) = result.stdout_lines.first() else {
        return Err(DurationError::ExpectedLine { result });
    };

    let duration_seconds = duration_line
        .parse::<f64>()
        .map_err(|e| DurationError::Parse {
            inner_error: e.into(),
        })?;

    Ok(Duration::from_secs_f64(duration_seconds))
}
