use libcmd::{CommandExit, CommandMonitorServer};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use valuable::Valuable;

use crate::ffmpeg::{error::FfmpegError, find::find_ffmpeg};

/// Run ffmpeg with output monitoring via a [`CommandMonitorServer`].
///
/// Stdout and stderr lines are streamed through the monitor, allowing
/// real-time progress parsing or logging. On cancellation the process
/// is killed immediately — use [`super::ffmpeg_graceful`] if you need
/// stdin-based quit with a SIGKILL fallback.
#[instrument(skip_all)]
pub async fn ffmpeg<Prepare>(
    cancellation_token: CancellationToken,
    server: &CommandMonitorServer,
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

    libcmd::run(
        ffmpeg_path,
        Some(server.clone()),
        cancellation_token.child_token(),
        prepare,
    )
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
