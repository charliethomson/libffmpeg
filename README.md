# libffmpeg

Async Rust wrapper for ffmpeg and ffprobe, built on tokio, with tracing and graceful shutdown support.

## Features

- **Three ffmpeg execution modes**: slim (no monitoring), standard (with monitoring), and graceful (with stdin-based quit and SIGKILL fallback)
- **ffprobe support**: run ffprobe commands and extract media metadata (e.g. duration)
- **Async-first**: built on `tokio` with `CancellationToken` for cancellation
- **Progress parsing**: structured `Progress` type parsed from ffmpeg's `-progress pipe:1` output
- **Tracing integration**: all functions are instrumented with `tracing` spans and `valuable` support
- **Binary discovery**: host-set paths, env var overrides, `$PATH`, platform install locations, and an opt-in login-shell fallback — with `tools::locate` reporting which one answered

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

`ffmpeg`, `ffprobe` and `ffplay` are located automatically. The lookup order is:

1. A path set by the host application via `tools::set_tool_path`
2. The tool's environment variable (`LIBFFMPEG_FFMPEG_PATH`, `LIBFFMPEG_FFPROBE_PATH`, `LIBFFMPEG_FFPLAY_PATH`)
3. `$PATH`, via `libwhich`
4. Well-known install locations for the platform (Homebrew/MacPorts on macOS; winget, Program Files and Chocolatey on Windows)
5. The *login* shell's `$PATH` — unix only, opt-in via `LIBFFMPEG_USE_LOGIN_SHELL_PATH`

Nothing is cached, so a path set at runtime — or an ffmpeg installed while the
app is open — takes effect on the next call.

```bash
export LIBFFMPEG_FFMPEG_PATH=/opt/homebrew/bin/ffmpeg
export LIBFFMPEG_FFPROBE_PATH=/opt/homebrew/bin/ffprobe
```

### Configuring paths from an application

Prefer `set_tool_path` over the environment variables. Setting an env var from
inside the process means `std::env::set_var`, which is `unsafe` on edition 2024
and unsound once any thread exists — so it can only be done at the very top of
`main`. `set_tool_path` has neither restriction, which is what lets an app apply
a path the user typed into a settings screen.

```rust
use libffmpeg::tools::{self, Tool};

// Apply the user's configured path (None clears it and restores the search).
tools::set_tool_path(Tool::Ffmpeg, config.ffmpeg_path.clone());
```

### Reporting status

`locate` returns both the resolved path and *how* it was found, so an
application can tell the user what it's about to run instead of letting a
missing binary surface as a spawn failure:

```rust
use libffmpeg::tools::{self, Tool};

match tools::locate(Tool::Ffmpeg) {
    Some(found) => println!("ffmpeg: {} ({})", found.path.display(), found.source.label()),
    None => {
        let (command, url) = tools::install_hint();
        println!("ffmpeg not found — install it: {}", command.unwrap_or(url));
    }
}
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

### `tools` module

| Item | Description |
|---|---|
| `Tool` | `Ffmpeg` / `Ffprobe` / `Ffplay`, with `binary_name()` and `env_key()` |
| `locate(tool)` | Resolve a tool, reporting the path *and* how it was found |
| `find(tool)` | Resolve a tool to a path |
| `set_tool_path(tool, path)` | Override a tool's location; beats the environment, no `unsafe` |
| `tool_path_override(tool)` | Read back the current override |
| `platform_default_dirs()` | Well-known install locations for this platform |
| `install_hint()` | Suggested install command + download URL for this platform |

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
