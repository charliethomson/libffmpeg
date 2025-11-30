# libffmpeg

Wrapper crate for ffmpeg, or any other command, async first, built on tokio, with tracing.

## Features

- Async command execution with `tokio`
- Built-in cancellation support via `CancellationToken`
- Progress monitoring for long-running ffmpeg operations
- Tracing integration for observability
- Generic command runner that works with any CLI tool

## Installation

```toml
[dependencies]
libffmpeg = { git = "https://github.com/charliethomson/libffmpeg" }
```

## Usage

### Environment Variables

Both `ffmpeg` and `duration` modules use `find_binary_env()` to locate binaries. Set these if ffmpeg/ffprobe aren't on your `$PATH`:

- `LIBFFMPEG_FFMPEG_PATH` - path to ffmpeg binary
- `LIBFFMPEG_FFPROBE_PATH` - path to ffprobe binary

```bash
export LIBFFMPEG_FFMPEG_PATH=/opt/homebrew/bin/ffmpeg
export LIBFFMPEG_FFPROBE_PATH=/opt/homebrew/bin/ffprobe
```

`find_binary_env()` checks the env var first, validates it (exists, executable), then falls back to searching `$PATH`.

### Setup
Copy `.cargo/config.toml` into your workspace, `tracing` [still](https://github.com/tokio-rs/tracing/discussions/1906) hasnt made `valuable` support stable :/

#### with curl
```bash
mkdir -p .cargo && curl https://raw.githubusercontent.com/charliethomson/ffrenc/refs/heads/main/.cargo/config.toml > .cargo/config.toml
```

#### with wget
```bash
mkdir -p .cargo && wget https://raw.githubusercontent.com/charliethomson/ffrenc/refs/heads/main/.cargo/config.toml -O .cargo/config.toml
```

### Basic ffmpeg execution

```rust
use libffmpeg::ffmpeg::ffmpeg;
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let result = ffmpeg(token, |cmd| {
    cmd.arg("-i").arg("input.mp4")
       .arg("-c:v").arg("libx264")
       .arg("output.mp4");
}).await?;
```

### With progress monitoring
[real example](https://github.com/charliethomson/ffrenc/blob/main/src/tasks.rs#L104)
```rust
use libffmpeg::ffmpeg::ffmpeg_with_progress;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

let (tx, mut rx) = mpsc::channel(100);
let token = CancellationToken::new();

tokio::spawn(async move {
    while let Some(duration) = rx.recv().await {
        println!("Progress: {:?}", duration);
    }
});

let result = ffmpeg_with_progress(tx, token, |cmd| {
    cmd.arg("-i").arg("input.mp4")
       .arg("output.mp4");
}).await?;
```

### Generic command runner

```rust
use libffmpeg::util::cmd::run;
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let result = run("ls", None, token, |cmd| {
    cmd.arg("-la");
}).await?;
```

## API

- `ffmpeg()` - Run ffmpeg with cancellation support
- `ffmpeg_with_progress()` - Run ffmpeg and receive progress updates via channel
- `util::cmd::run()` - Generic command runner for any CLI tool

All functions accept a `CancellationToken` for graceful shutdown and a closure to configure the command.

## License

dont care dont sue me, its 500 lines of wrapper code
