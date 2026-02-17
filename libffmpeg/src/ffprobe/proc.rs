use libcmd::CommandExit;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use valuable::Valuable;

use crate::ffprobe::{error::FfprobeError, find::find_ffprobe};

/// Run ffprobe with cancellation support.
///
/// Locates the ffprobe binary (via `LIBFFMPEG_FFPROBE_PATH` or `$PATH`),
/// spawns it with the arguments configured by `prepare`, and returns
/// the captured stdout/stderr and exit code.
#[instrument(skip(prepare, cancellation_token))]
pub async fn ffprobe<Prepare>(
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, FfprobeError>
where
    Prepare: FnOnce(&mut Command),
{
    tracing::debug!("Starting ffprobe execution");

    let ffprobe_path = find_ffprobe().ok_or(FfprobeError::NotFound).inspect_err(
        |e| tracing::error!(error =% e, error_context =? e, "ffprobe binary not found"),
    )?;

    tracing::info!(
        ffprobe_path = %ffprobe_path.display(),
        "Executing ffprobe"
    );

    libcmd::run(
        ffprobe_path,
        None,
        cancellation_token.child_token(),
        prepare,
    )
    .await
    .inspect(|exit| {
        tracing::debug!(exit = exit.as_value(), "ffprobe completed");
    })
    .inspect_err(|e| {
        tracing::error!(
            error = %e,
            "ffprobe execution failed"
        );
    })
    .map_err(Into::into)
}
