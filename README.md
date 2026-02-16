# libffmpeg

Async Rust wrapper for ffmpeg and ffprobe, built on tokio, with tracing and graceful shutdown support.

## Features

- **Three ffmpeg execution modes**: slim (no monitoring), standard (with monitoring), and graceful (with stdin-based quit and SIGKILL fallback)
- **ffprobe support**: run ffprobe commands and extract media metadata (e.g. duration)
- **Async-first**: built on `tokio` with `CancellationToken` for cancellation
- **Progress parsing**: structured `Progress` type parsed from ffmpeg's `-progress pipe:1` output
- **Tracing integration**: all functions are instrumented with `tracing` spans and `valuable` support
- **Binary discovery**: uses `libwhich` with env var overrides (`LIBFFMPEG_FFMPEG_PATH`, `LIBFFMPEG_FFPROBE_PATH`)

## Installation

```toml
[dependencies]
libffmpeg = { git = "https://github.com/charliethomson/libffmpeg" }
```

### Setup

Copy `.cargo/config.toml` into your workspace to enable `tracing`'s unstable `valuable` support:

```bash
mkdir -p .cargo && curl -o .cargo/config.toml https://raw.githubusercontent.com/charliethomson/libffmpeg/refs/heads/main/.cargo/config.toml
```

## Binary Discovery

Both `ffmpeg` and `ffprobe` are located automatically. For each binary, the lookup order is:

1. Check the environment variable (`LIBFFMPEG_FFMPEG_PATH` / `LIBFFMPEG_FFPROBE_PATH`) and validate it's an executable
2. Fall back to searching `$PATH` via `libwhich`

```bash
export LIBFFMPEG_FFMPEG_PATH=/opt/homebrew/bin/ffmpeg
export LIBFFMPEG_FFPROBE_PATH=/opt/homebrew/bin/ffprobe
```

## Usage

### ffmpeg execution modes

#### `ffmpeg_slim` — no monitoring

Runs ffmpeg with cancellation support only. No `CommandMonitorServer` needed.

```rust
use libffmpeg::ffmpeg::ffmpeg_slim;
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let result = ffmpeg_slim(token, |cmd| {
    cmd.arg("-i").arg("input.mp4")
       .arg("-c:v").arg("libx264")
       .arg("output.mp4");
}).await?;
```

#### `ffmpeg` — with monitoring

Runs ffmpeg with a `CommandMonitorServer` for stdout/stderr streaming. Use this when you want to receive output lines (e.g. for progress parsing).

```rust
use libffmpeg::ffmpeg::ffmpeg;
use libffmpeg::libcmd::CommandMonitor;
use tokio_util::sync::CancellationToken;

let monitor = CommandMonitor::with_capacity(100);
let token = CancellationToken::new();

let result = ffmpeg(token, &monitor.server, |cmd| {
    cmd.arg("-i").arg("input.mp4")
       .arg("-progress").arg("pipe:1")
       .arg("-y")
       .arg("output.mp4");
}).await?;
```

#### `ffmpeg_graceful` — with graceful shutdown

Like `ffmpeg`, but on cancellation it sends `q` to ffmpeg's stdin first, giving it up to 5 seconds to exit cleanly before falling back to SIGKILL.

```rust
use libffmpeg::ffmpeg::ffmpeg_graceful;
use libffmpeg::libcmd::CommandMonitor;
use tokio_util::sync::CancellationToken;

let monitor = CommandMonitor::with_capacity(100);
let token = CancellationToken::new();

let result = ffmpeg_graceful(token, &monitor.client, &monitor.server, |cmd| {
    cmd.arg("-i").arg("input.mp4")
       .arg("-c:v").arg("libx264")
       .arg("output.mp4");
}).await?;
```

### ffprobe

```rust
use libffmpeg::ffprobe::ffprobe;
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let result = ffprobe(token, |cmd| {
    cmd.arg("-v").arg("quiet")
       .arg("-show_entries").arg("format=duration")
       .arg("-of").arg("default=noprint_wrappers=1:nokey=1")
       .arg("input.mp4");
}).await?;
```

### Get duration

A convenience function that wraps ffprobe to extract a file's duration:

```rust
use libffmpeg::util::get_duration;
use tokio_util::sync::CancellationToken;

let duration = get_duration("input.mp4", CancellationToken::new()).await?;
println!("Duration: {:?}", duration);
```

### Progress parsing

When using ffmpeg with `-progress pipe:1`, you can parse the output into structured `Progress` updates:

```rust
use libffmpeg::ffmpeg::progress::PartialProgress;

let mut progress = PartialProgress::default();
// Feed lines from ffmpeg's stdout
if progress.with_line("frame=100") {
    if let Some(update) = progress.finish() {
        println!("Frame: {}, FPS: {}, Speed: {}x", update.frame, update.fps, update.speed);
    }
}
```

See [`examples/transcode_with_progress.rs`](libffmpeg/examples/transcode_with_progress.rs) for a full working example.

## API Reference

### `ffmpeg` module

| Function | Description |
|---|---|
| `ffmpeg_slim(token, prepare)` | Run ffmpeg with cancellation only |
| `ffmpeg(token, server, prepare)` | Run ffmpeg with monitoring |
| `ffmpeg_graceful(token, client, server, prepare)` | Run ffmpeg with graceful shutdown |

### `ffprobe` module

| Function | Description |
|---|---|
| `ffprobe(token, prepare)` | Run ffprobe with cancellation |

### `util` module

| Function | Description |
|---|---|
| `get_duration(path, token)` | Get media duration via ffprobe |

### `ffmpeg::progress` module

| Type | Description |
|---|---|
| `Progress` | Structured progress update (frame, fps, bitrate, speed, etc.) |
| `PartialProgress` | Accumulator for parsing ffmpeg progress lines |
| `ProgressState` | Continue, End, or Unknown |

## Dependencies

This crate uses several companion libraries:

- [`libcmd`](https://github.com/charliethomson/libcmd) — async command execution with monitoring and cancellation
- [`libwhich`](https://github.com/charliethomson/which) — binary discovery
- [`liberror`](https://github.com/charliethomson/liberror) — error utilities
- [`libsignal`](https://github.com/charliethomson/libsignal) — signal handling (used in examples)

## License

dont care dont sue me, its a wrapper crate
