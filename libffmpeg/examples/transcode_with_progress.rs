use std::{path::PathBuf, time::Duration};

use clap::Parser;
use libffmpeg::ffmpeg::ffmpeg;
use tokio_util::{future::FutureExt, sync::CancellationToken};

#[derive(Debug, Parser)]
struct Args {
    #[arg(short, long)]
    input: PathBuf,

    #[arg(short, long)]
    output: PathBuf,

    #[arg(short, long)]
    time: Option<u64>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let root_token = CancellationToken::new();
    let transcode_token = root_token.child_token();
    let exit_token = root_token.child_token();
    libsignal::cancel_after_signal(root_token.clone());

    let video_duration =
        libffmpeg::util::get_duration(&args.input, root_token.child_token()).await?;

    let total = args.time.unwrap_or(video_duration.as_secs()) as f64;

    let monitor = libffmpeg::libcmd::CommandMonitor::with_capacity(100);

    let monitor_fut = {
        let mut client = monitor.client.clone();
        let exit_token = exit_token.clone();
        tokio::spawn(async move {
            let mut progress = libffmpeg::ffmpeg::progress::PartialProgress::default();

            while let Some(Some(delivery)) =
                client.recv().with_cancellation_token(&exit_token).await
            {
                match delivery {
                    libffmpeg::libcmd::CommandMonitorMessage::Stdout { line } => {
                        if !progress.with_line(&line) {
                            println!("{}[O] {}{}", "\x1b[32m", line, "\x1b[0m");
                            continue;
                        }

                        if let Some(update) = progress.finish() {
                            println!(
                                "{}[P] {:.2}% ({} / {}); bitrate={} @ {}fps ({:.1}x realtime) {}",
                                "\x1b[33m",
                                100f64 * update.out_time.as_secs_f64() / total,
                                humantime::format_duration(update.out_time),
                                humantime::format_duration(Duration::from_secs_f64(total)),
                                humansize::format_size_i(
                                    update.bitrate,
                                    humansize::DECIMAL.suffix("ps")
                                ),
                                update.fps,
                                update.speed,
                                "\x1b[0m"
                            );
                        }
                    }
                    libffmpeg::libcmd::CommandMonitorMessage::Stderr { line } => {
                        eprintln!("{}[E] {}{}", "\x1b[31m", line, "\x1b[0m")
                    }
                }
            }
        })
    };

    let result = ffmpeg(transcode_token, &monitor.server, |cmd| {
        cmd.arg("-i").arg(&args.input);
        cmd.arg("-t").arg(total.to_string());
        cmd.arg("-c:v").arg("libx264");
        cmd.arg("-preset").arg("fast");
        cmd.arg("-crf").arg("23");
        cmd.arg("-c:a").arg("aac");
        cmd.arg("-b:a").arg("128k");
        cmd.arg("-progress").arg("pipe:1");
        cmd.arg("-y");
        cmd.arg(&args.output);
    })
    .await?;

    exit_token.cancel();

    if let Err(e) = monitor_fut.await {
        eprintln!("Failed to wait for monitor: {}", e);
    }

    println!("");

    if result
        .exit_code
        .as_ref()
        .map(|exit| exit.success)
        .unwrap_or_default()
    {
        println!("Transcoding completed successfully");
    } else {
        println!("Transcoding failed: {:#?}", result);
    }

    Ok(())
}
