use std::{path::Path, time::Duration};

use liberror::AnyError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use valuable::Valuable;

use libcmd::{CommandExit, CommandExitCode};

use crate::ffprobe::{FfprobeError, ffprobe};

/// Errors that can occur when extracting a media file's duration via ffprobe.
#[derive(Error, Debug, Clone, Serialize, Deserialize, Valuable)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "$type",
    content = "context"
)]
pub enum DurationError {
    #[serde(rename = "dev.thmsn.ffmpeg.duration.error.ffprobe")]
    #[error(transparent)]
    Ffprobe {
        #[from]
        inner_error: FfprobeError,
    },

    #[serde(rename = "dev.thmsn.ffmpeg.duration.error.incomplete_subprocess")]
    #[error("Process returned, but no exit status was present: stdout_lines={}, stderr_lines={}", result.stdout_lines.len(), result.stderr_lines.len())]
    IncompleteSubprocess { result: CommandExit },
    #[serde(rename = "dev.thmsn.ffmpeg.duration.error.exited_unsuccessfully")]
    #[error("ffprobe exited unsuccessfully with code {}: {:?}", exit_code.code.map_or_else(|| "unknown".to_string(), |c| c.to_string()), exit_code)]
    ExitedUnsuccessfully { exit_code: CommandExitCode },
    #[serde(rename = "dev.thmsn.ffmpeg.duration.error.expected_line")]
    #[error("Expected ffprobe to output a line with the duration, got {} stdout lines and {} stderr lines", result.stdout_lines.len(), result.stderr_lines.len())]
    ExpectedLine { result: CommandExit },
    #[serde(rename = "dev.thmsn.ffmpeg.duration.error.parse")]
    #[error("Failed to parse duration provided by ffprobe: {inner_error}")]
    Parse { inner_error: AnyError },
}

/// Extract the duration of a media file using ffprobe.
///
/// Runs ffprobe in quiet mode and parses the `format=duration` entry
/// from the output. Returns the duration as a [`std::time::Duration`].
///
/// A DEBUG span: a duration probe is per-item work inside the caller's unit.
#[instrument(name = "ffprobe.duration", level = "debug", skip(input, cancellation_token), fields(input_path = %input.as_ref().display()))]
#[allow(clippy::too_many_lines)]
pub async fn get_duration<P: AsRef<Path>>(
    input: P,
    cancellation_token: CancellationToken,
) -> Result<Duration, DurationError> {
    tracing::debug!(
        input_path = %input.as_ref().display(),
        "Executing ffprobe to get duration"
    );

    let mut result = ffprobe(cancellation_token, move |cmd| {
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
        return Err(DurationError::ExitedUnsuccessfully { exit_code });
    }

    let Some(duration_line) = result.stdout_lines.first() else {
        return Err(DurationError::ExpectedLine { result });
    };

    tracing::trace!(
        duration_line = %duration_line,
        "Parsing duration from ffprobe output"
    );

    let duration_seconds = duration_line
        .parse::<f64>()
        .map_err(|e| DurationError::Parse {
            inner_error: e.into(),
        })
        .inspect(|seconds| {
            tracing::trace!(
                duration_seconds = %seconds,
                "Successfully parsed duration"
            );
        })?;

    let duration = Duration::from_secs_f64(duration_seconds);

    tracing::debug!(
        duration_seconds = %duration_seconds,
        "Successfully extracted duration"
    );

    Ok(duration)
}
