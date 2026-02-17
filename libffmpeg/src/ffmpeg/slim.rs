use libcmd::CommandExit;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use valuable::Valuable;

use crate::ffmpeg::{error::FfmpegError, find::find_ffmpeg};

/// Run ffmpeg with cancellation support only (no output monitoring).
///
/// This is the lightest-weight variant — it spawns ffmpeg, waits for it to
/// complete, and returns the exit result. Use [`super::ffmpeg`] if you need
/// to stream stdout/stderr, or [`super::ffmpeg_graceful`] for graceful shutdown.
#[instrument(skip(prepare, cancellation_token))]
pub async fn ffmpeg_slim<Prepare>(
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, FfmpegError>
where
    Prepare: FnOnce(&mut Command),
{
    tracing::debug!("Starting ffmpeg execution");

    let ffmpeg_path = find_ffmpeg().ok_or(FfmpegError::NotFound).inspect_err(
        |e| tracing::error!(error =% e, error_context =? e, "ffmpeg binary not found"),
    )?;

    tracing::info!(
        ffmpeg_path = %ffmpeg_path.display(),
        "Executing ffmpeg"
    );

    libcmd::run(ffmpeg_path, None, cancellation_token.child_token(), prepare)
        .await
        .inspect(|exit| {
            tracing::debug!(exit = exit.as_value(), "ffmpeg completed");
        })
        .inspect_err(|e| {
            tracing::error!(
                error = %e,
                "ffmpeg execution failed"
            );
        })
        .map_err(Into::into)
}
