use std::time::Duration;

use tokio::process::Command;
use tokio_util::{future::FutureExt, sync::CancellationToken};

use crate::util::cmd::{self, CommandError, CommandExit, CommandMonitor};

pub async fn ffmpeg<Prepare>(
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, CommandError>
where
    Prepare: FnOnce(&mut Command),
{
    return cmd::run("ffmpeg", None, cancellation_token.child_token(), prepare).await;
}

/// NOTE: This adds `-progress pipe:1 -hide_banner -loglevel error` to the BEGINNING of the `prepare`d command
#[tracing::instrument("libffmpeg::ffmpeg::progress", skip(prepare))]
pub async fn ffmpeg_with_progress<Prepare>(
    tx: tokio::sync::mpsc::Sender<Duration>,
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, CommandError>
where
    Prepare: FnOnce(&mut Command),
{
    let mut monitor = CommandMonitor::with_capacity(100);

    // TODO: support overriding ffmpeg/ffprobe paths
    let fut = cmd::run(
        "ffmpeg",
        Some(monitor.sender),
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
            loop {
                let delivery = match monitor.receiver.recv().with_cancellation_token(&monitor_token).await {
                Some(Some(delivery)) => delivery,
                Some(None) /* closed */ => break,
                None /* cancelled */ => break,
            };

                match delivery {
                    cmd::CommandMonitorMessage::Stdout { line } => {
                        if !line.starts_with("out_time_us") {
                            continue;
                        }
                        let Some(duration_us) = line.split_once('=').map(|x| x.1) else {
                            continue;
                        };
                        let Ok(duration_us) = duration_us.parse::<f64>() else {
                            continue;
                        };

                        let duration_seconds = duration_us / 1_000_000.0;
                        if duration_seconds < f64::EPSILON {
                            continue;
                        }
                        if let Err(e) = tx.send(Duration::from_secs_f64(duration_seconds)).await {
                            tracing::warn!("Failed to send progress: {e}");
                        }
                    }
                    // dont care!
                    cmd::CommandMonitorMessage::Stderr { line } => drop(line),
                }
            }
        })
    };

    let result = fut.await;
    monitor_token.cancel();
    // This should never block, but w/e :)
    if let Err(_timeout) = tokio::time::timeout(Duration::from_millis(500), handle).await {
        tracing::warn!("Timed out waiting for monitor to close");
    }
    result
}
