use std::time::Duration;

use libcmd::{CommandError, CommandExit, CommandMonitorClient, CommandMonitorServer};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, instrument};
use valuable::Valuable;

use crate::ffmpeg::{error::FfmpegError, find::find_ffmpeg};

/// Run ffmpeg with output monitoring and graceful shutdown.
///
/// On cancellation, sends `q` to ffmpeg's stdin (its built-in quit command),
/// giving the process up to 5 seconds to exit cleanly before falling back to
/// SIGKILL. This allows ffmpeg to finalize the output file properly.
#[instrument(name = "ffmpeg.run", skip_all, fields(otel.status_code = tracing::field::Empty))]
pub async fn ffmpeg_graceful<Prepare>(
    cancellation_token: CancellationToken,
    client: &CommandMonitorClient,
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

    // Different source token for the process, lets us gracefully exit
    let process_token = CancellationToken::new();

    // Cancelled after the process exits
    let exit_token = CancellationToken::new();

    // Flow:
    //  1. If the process exits naturally before cancellation, do nothing and return early
    //  2. User requests cancellation
    //  3. Send "q" to ffmpeg's stdin
    //  4. Give the process a max of 5 seconds to exit (wait using `exit_token`, quit should tell the process to exit normally)
    //  5. If the process doesn't exit after 5 seconds, cancel the process' token, signals that it should send SIGKILL
    //  6. The process will be killed, as if none of this was ever here
    //
    // The watcher lives as long as the process, so it gets no span of its own
    // (a process-lifetime span never finishes inside a trace window); its rare
    // events attach to `ffmpeg.run`.
    let shutdown_handle = {
        let client = client.clone();
        let process_token = process_token.clone();
        let exit_token = exit_token.clone();
        let kill_token = cancellation_token.child_token();
        tokio::spawn(
            async move {
                // Wait for kill token to cancel (user requested cancellation)
                tokio::select! {
                    () = exit_token.cancelled() => {
                        // if process exits before kill is requested, we don't want to kill the process
                        return
                    },
                    () = kill_token.cancelled() => {
                        // Continue killing the process
                    }
                }

                // Send quit
                tracing::debug!("graceful shutdown requested, sending q to ffmpeg");
                client.send("q").await;

                // Wait for exit to be cancelled (process exited), with max of 5 seconds
                match tokio::time::timeout(Duration::from_secs(5), exit_token.cancelled()).await {
                    Ok(()) => {}
                    Err(_timeout) => {
                        // Process didn't respond to quit command, tell the manager to kill the process
                        tracing::warn!(
                            "ffmpeg process did not respond to quit command, sending SIGKILL"
                        );
                        process_token.cancel();
                    }
                }
            }
            .in_current_span(),
        )
    };

    let result = libcmd::run(
        ffmpeg_path,
        Some(server.clone()),
        process_token.child_token(),
        prepare,
    )
    .await
    .inspect(|exit| {
        tracing::debug!(exit = exit.as_value(), "ffmpeg completed");
    })
    .inspect_err(|e| {
        // Graceful shutdown ends in a cancellation (the SIGKILL fallback after
        // ffmpeg ignores `q`). That is the intended stall-kill path, not a
        // failure — keep it out of the error stream.
        if let CommandError::Cancelled = e {
            tracing::debug!("ffmpeg execution cancelled (graceful shutdown)");
        } else {
            tracing::Span::current().record("otel.status_code", "ERROR");
            tracing::error!(error = %e, "ffmpeg execution failed");
        }
    })
    .map_err(Into::into);

    exit_token.cancel();

    if let Err(e) = shutdown_handle.await {
        tracing::error!(error=%e, error_context=?e,"Failed to wait for shutdown handle to exit");
    }

    result
}
