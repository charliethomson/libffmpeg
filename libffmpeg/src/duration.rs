use std::{path::Path, time::Duration};

use liberror::AnyError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use valuable::Valuable;

use crate::env::find::{FindBinaryError, find_binary_env};

use libcmd::{CommandError, CommandExit, CommandExitCode};

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

    #[error("Process returned, but no exit status was present: stdout_lines={}, stderr_lines={}", result.stdout_lines.len(), result.stderr_lines.len())]
    IncompleteSubprocess { result: CommandExit },
    #[error("ffprobe exited unsuccessfully with code {}: {:?}", exit_code.code.map_or_else(|| "unknown".to_string(), |c| c.to_string()), exit_code)]
    ExitedUnsuccessfully { exit_code: CommandExitCode },
    #[error("Expected ffprobe to output a line with the duration, got {} stdout lines and {} stderr lines: {}", result.stdout_lines.len(), result.stderr_lines.len(), result.stdout_lines.join("\n"))]
    ExpectedLine { result: CommandExit },
    #[error("Failed to parse duration provided by ffprobe: {inner_error}")]
    Parse { inner_error: AnyError },
    #[error(
        "Unable to locate ffprobe on your PATH, set LIBFFMPEG_FFPROBE_PATH to the binary, or update your PATH"
    )]
    FfprobeNotFound,
}

#[instrument(skip(input, cancellation_token), fields(input_path = %input.as_ref().display()))]
#[allow(clippy::too_many_lines)]
pub async fn get_duration<P: AsRef<Path>>(
    input: P,
    cancellation_token: CancellationToken,
) -> Result<Duration, DurationError> {
    tracing::debug!(
        input_path = %input.as_ref().display(),
        "Starting duration extraction"
    );

    let Some(ffprobe_path) = find_binary_env("ffprobe").await.inspect_err(|e| {
        tracing::error!(
            error = %e,
            "Failed to search for ffprobe binary"
        );
    })?
    else {
        tracing::error!("ffprobe binary not found");
        return Err(DurationError::FfprobeNotFound);
    };

    tracing::info!(
        ffprobe_path = %ffprobe_path.display(),
        input_path = %input.as_ref().display(),
        "Executing ffprobe to get duration"
    );

    let mut result = libcmd::run(ffprobe_path, None, cancellation_token, move |cmd| {
        cmd.arg("-threads").arg("4");
        cmd.arg("-v").arg("quiet");
        cmd.arg("-show_entries").arg("format=duration");
        cmd.arg("-of").arg("default=noprint_wrappers=1:nokey=1");
        cmd.arg(input.as_ref());
    })
    .await
    .inspect(|exit| {
        tracing::debug!(
            exit_code = ?exit.exit_code,
            stdout_lines = exit.stdout_lines.len(),
            stderr_lines = exit.stderr_lines.len(),
            "ffprobe completed"
        );
    })
    .inspect_err(|e| {
        tracing::error!(
            error = %e,
            "ffprobe execution failed"
        );
    })?;

    let Some(exit_code) = result.exit_code.take() else {
        tracing::error!(
            stdout_lines = result.stdout_lines.len(),
            stderr_lines = result.stderr_lines.len(),
            "Process returned but no exit status was present"
        );
        return Err(DurationError::IncompleteSubprocess { result });
    };

    if !exit_code.success {
        tracing::error!(
            exit_code = ?exit_code,
            stderr_lines = ?result.stderr_lines,
            "ffprobe exited unsuccessfully"
        );
        return Err(DurationError::ExitedUnsuccessfully { exit_code });
    }

    let Some(duration_line) = result.stdout_lines.first() else {
        tracing::error!(
            stdout_lines = ?result.stdout_lines,
            stderr_lines = ?result.stderr_lines,
            "Expected ffprobe to output a line with the duration"
        );
        return Err(DurationError::ExpectedLine { result });
    };

    tracing::trace!(
        duration_line = %duration_line,
        "Parsing duration from ffprobe output"
    );

    let duration_seconds = duration_line
        .parse::<f64>()
        .map_err(|e| {
            tracing::error!(
                duration_line = %duration_line,
                error = %e,
                "Failed to parse duration from ffprobe output"
            );
            DurationError::Parse {
                inner_error: e.into(),
            }
        })
        .inspect(|seconds| {
            tracing::trace!(
                duration_seconds = %seconds,
                "Successfully parsed duration"
            );
        })?;

    let duration = Duration::from_secs_f64(duration_seconds);

    tracing::info!(
        duration_seconds = %duration_seconds,
        "Successfully extracted duration"
    );

    Ok(duration)
}
