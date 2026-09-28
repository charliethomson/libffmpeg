use libcmd::{CommandError, CommandExit, CommandMonitorServer};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use valuable::Valuable;

use crate::ffmpeg::{error::FfmpegError, find::find_ffmpeg};

/// Run ffmpeg with output monitoring via a [`CommandMonitorServer`].
///
/// Stdout and stderr lines are streamed through the monitor, allowing
/// real-time progress parsing or logging. On cancellation the process
/// is killed immediately — use [`super::ffmpeg_graceful`] if you need
/// stdin-based quit with a SIGKILL fallback.
pub async fn ffmpeg<Prepare>(
    cancellation_token: CancellationToken,
    server: &CommandMonitorServer,
    prepare: Prepare,
) -> Result<CommandExit, FfmpegError>
where
    Prepare: FnOnce(&mut Command),
{
    // Built by hand (not `#[instrument]`) so the body holds the handle:
    // `otel.status_code` is recorded on this span, which is a no-op when it's
    // filtered out, never on whatever span the caller has current.
    let span = tracing::debug_span!("ffmpeg.monitored", otel.status_code = tracing::field::Empty);
    async {
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
            // Cancellation kills the process on purpose — not a failure.
            if let CommandError::Cancelled = e {
                tracing::debug!("ffmpeg execution cancelled");
            } else {
                span.record("otel.status_code", "ERROR");
                tracing::error!(error = %e, "ffmpeg execution failed");
            }
        })
        .map_err(Into::into)
    }
    .instrument(span.clone())
    .await
}
