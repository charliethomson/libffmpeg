use std::time::Duration;

use libcmd::{
    CommandError, CommandExit, CommandMonitor, CommandMonitorClient, CommandMonitorServer,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;
use tokio_util::{future::FutureExt, sync::CancellationToken};
use tracing::instrument;
use valuable::Valuable;

use crate::env::find::{FindBinaryError, find_binary_env};

#[derive(Debug, Clone, Serialize, Deserialize, Valuable, Error)]
pub enum FfmpegError {
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
    #[error(
        "Unable to locate ffmpeg on your PATH, set LIBFFMPEG_FFMPEG_PATH to the binary, or update your PATH"
    )]
    NotFound,
}

#[instrument(skip(prepare, cancellation_token))]
pub async fn ffmpeg<Prepare>(
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, FfmpegError>
where
    Prepare: FnOnce(&mut Command),
{
    tracing::debug!("Starting ffmpeg execution");

    let Some(ffmpeg_path) = find_binary_env("ffmpeg").await.inspect_err(|e| {
        tracing::error!(
            error = %e,
            "Failed to search for ffmpeg binary"
        );
    })?
    else {
        tracing::error!("ffmpeg binary not found");
        return Err(FfmpegError::NotFound);
    };

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

/// NOTE: This adds `-progress pipe:1 -hide_banner -loglevel error` to the BEGINNING of the `prepare`d command
#[tracing::instrument("libffmpeg::ffmpeg::progress", skip(prepare, tx, cancellation_token))]
#[allow(clippy::too_many_lines)]
pub async fn ffmpeg_with_progress<Prepare>(
    tx: tokio::sync::mpsc::Sender<Duration>,
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, FfmpegError>
where
    Prepare: FnOnce(&mut Command),
{
    tracing::debug!("Starting ffmpeg execution");

    let Some(ffmpeg_path) = find_binary_env("ffmpeg").await.inspect_err(|e| {
        tracing::error!(
            error = %e,
            "Failed to search for ffmpeg binary"
        );
    })?
    else {
        tracing::error!("ffmpeg binary not found");
        return Err(FfmpegError::NotFound);
    };

    tracing::info!(
        ffmpeg_path = %ffmpeg_path.display(),
        "Executing ffmpeg"
    );

    let mut monitor = CommandMonitor::with_capacity(100);

    let fut = libcmd::run(
        ffmpeg_path,
        Some(monitor.server),
        cancellation_token.child_token(),
        |cmd| {
            cmd.arg("-hide_banner");
            cmd.arg("-progress").arg("pipe:1");
            cmd.arg("-loglevel").arg("error");
            prepare(cmd);
        },
    );

    let monitor_token = cancellation_token.child_token();
    let handle = {
        let monitor_token = monitor_token.clone();
        tokio::spawn(async move {
            tracing::debug!("Starting progress monitor loop");
            loop {
                let delivery = match monitor.client.recv().with_cancellation_token(&monitor_token).await {
                Some(Some(delivery)) => delivery,
                Some(None) /* closed */ => {
                    tracing::debug!("Progress monitor channel closed");
                    break;
                },
                None /* cancelled */ => {
                    tracing::debug!("Progress monitor cancelled");
                    break;
                },
            };

                match delivery {
                    libcmd::CommandMonitorMessage::Stdout { line } => {
                        if !line.starts_with("out_time_us") {
                            continue;
                        }
                        let Some(duration_us) = line.split_once('=').map(|x| x.1) else {
                            tracing::trace!(line = %line, "Progress line missing '=' separator");
                            continue;
                        };
                        let Ok(duration_us) = duration_us.parse::<f64>() else {
                            tracing::warn!(duration_str = %duration_us, "Failed to parse progress duration");
                            continue;
                        };

                        let duration_seconds = duration_us / 1_000_000.0;
                        if duration_seconds < f64::EPSILON {
                            continue;
                        }

                        let duration = Duration::from_secs_f64(duration_seconds);
                        tracing::trace!(
                            duration_seconds = %duration_seconds,
                            "Sending progress update"
                        );

                        let _ = tx.send(duration).await.inspect_err(|e| {
                            tracing::warn!(
                                error = %e,
                                "Failed to send progress update to channel"
                            );
                        });
                    }
                    // dont care!
                    libcmd::CommandMonitorMessage::Stderr { line } => {
                        tracing::trace!(stderr_line = %line, "Received stderr from ffmpeg");
                        drop(line);
                    }
                }
            }
            tracing::debug!("Progress monitor loop completed");
        })
    };

    let result = fut.await;
    monitor_token.cancel();

    tracing::debug!("Waiting for progress monitor to shutdown");
    // This should never block, but w/e :)
    let _ = tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .inspect(|_| tracing::trace!("Progress monitor shutdown successfully"))
        .inspect_err(|_| tracing::warn!("Timed out waiting for progress monitor to close"));

    result
        .inspect(|exit| {
            tracing::debug!(
                exit_code = ?exit.exit_code,
                "ffmpeg with progress completed"
            );
        })
        .inspect_err(|e| {
            tracing::error!(
                error = %e,
                "ffmpeg with progress execution failed"
            );
        })
        .map_err(Into::into)
}

#[instrument(skip_all)]
pub async fn ffmpeg_graceful<Prepare>(
    cancellation_token: CancellationToken,
    client: &mut CommandMonitorClient,
    server: &mut CommandMonitorServer,
    prepare: Prepare,
) -> Result<CommandExit, FfmpegError>
where
    Prepare: FnOnce(&mut Command),
{
    tracing::debug!("Starting ffmpeg execution");

    let ffmpeg_path = find_binary_env("ffmpeg")
        .await
        .inspect_err(|e| tracing::error!(error = %e, "Failed to search for ffmpeg binary"))?
        .ok_or(FfmpegError::NotFound)
        .inspect_err(
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
    let kill_handle = {
        let client = client.clone();
        let process_token = process_token.clone();
        let exit_token = exit_token.clone();
        let kill_token = cancellation_token.child_token();
        // TODO: Instrument
        tokio::spawn(async move {
            // Wait for kill token to cancel (user requested cancellation)
            tokio::select! {
                _ = exit_token.cancelled() => {
                    // if process exits before kill is requested, we don't want to kill the process
                    return
                },
                _ = kill_token.cancelled() => {
                    // Continue killing the process
                }
            }

            // Send quit
            client.send("q").await;

            // Wait for exit to be cancelled (process exited), with max of 5 seconds
            match tokio::time::timeout(Duration::from_secs(5), exit_token.cancelled()).await {
                Ok(_) => {}
                Err(_timeout) => {
                    // Process didn't respond to quit command, tell the manager to kill the process
                    process_token.cancel();
                }
            }
        })
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
        tracing::error!(
            error = %e,
            "ffmpeg execution failed"
        );
    })
    .map_err(Into::into);

    exit_token.cancel();

    if let Err(e) = kill_handle.await {
        tracing::error!(error=%e, error_context=?e,"Failed to wait for kill handle to exit")
    };

    result
}
