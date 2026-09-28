//! A cancellation the caller asked for is not a failure (TTR-63), and the
//! per-item wrappers are DEBUG spans.
//!
//! One test fn: it points the tools at a hanging script through the
//! process-global override, and this file is its own test binary.

#![cfg(unix)]

use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use libffmpeg::{
    ffmpeg::ffmpeg_slim,
    ffprobe::ffprobe,
    tools::{Tool, set_tool_path},
    util::get_duration,
};
use tokio_util::sync::CancellationToken;
use tracing::{
    Event, Instrument, Level, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{
    Layer,
    filter::LevelFilter,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
};

#[derive(Default)]
struct Seen {
    /// `(span name, level)` for every span opened.
    spans: Vec<(String, Level)>,
    /// Every span a field was recorded on after creation.
    recorded_on: Vec<String>,
    /// Every event at WARN or above.
    loud: Vec<String>,
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Seen>>);

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Recorder {
    fn on_new_span(&self, attrs: &Attributes<'_>, _: &Id, _: Context<'_, S>) {
        let meta = attrs.metadata();
        self.0
            .lock()
            .unwrap()
            .spans
            .push((meta.name().to_string(), *meta.level()));
    }

    fn on_record(&self, id: &Id, _: &Record<'_>, ctx: Context<'_, S>) {
        let name = ctx.span(id).expect("span exists").name().to_string();
        self.0.lock().unwrap().recorded_on.push(name);
    }

    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() <= Level::WARN {
            let mut message = Message(String::new());
            event.record(&mut message);
            self.0.lock().unwrap().loud.push(message.0);
        }
    }
}

/// A script that runs until killed.
fn hanging_binary() -> PathBuf {
    let path = std::env::temp_dir().join(format!("libffmpeg-hang-{}", std::process::id()));
    std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A token that fires shortly after the process has spawned.
fn cancelled_soon() -> CancellationToken {
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    token
}

fn caller() -> tracing::Span {
    tracing::info_span!("caller", otel.status_code = tracing::field::Empty)
}

#[tokio::test]
async fn caller_cancellation_is_quiet_and_spans_are_debug() {
    let hang = hanging_binary();
    set_tool_path(Tool::Ffmpeg, Some(hang.clone()));
    set_tool_path(Tool::Ffprobe, Some(hang.clone()));

    let recorder = Recorder::default();
    let subscriber = tracing_subscriber::registry()
        .with(LevelFilter::TRACE)
        .with(recorder.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let slim = ffmpeg_slim(cancelled_soon(), |_| {})
        .instrument(caller())
        .await;
    assert!(slim.is_err(), "{slim:?}");
    let probe = ffprobe(cancelled_soon(), |_| {}).instrument(caller()).await;
    assert!(probe.is_err(), "{probe:?}");
    let duration = get_duration("input.mp4", cancelled_soon())
        .instrument(caller())
        .await;
    assert!(duration.is_err(), "{duration:?}");

    let seen = std::mem::take(&mut *recorder.0.lock().unwrap());

    // No ERROR "ffprobe execution failed" (or anything else loud) for a
    // cancellation the caller asked for, and no span marked ERROR.
    assert_eq!(seen.loud, Vec::<String>::new());
    assert!(
        !seen
            .recorded_on
            .iter()
            .any(|s| s.starts_with("ffmpeg") || s.starts_with("ffprobe") || s == "caller"),
        "{:?}",
        seen.recorded_on
    );

    // Per-item wrappers are DEBUG.
    for name in ["ffmpeg.slim", "ffprobe.run", "ffprobe.duration"] {
        let levels: Vec<Level> = seen
            .spans
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, l)| *l)
            .collect();
        assert!(!levels.is_empty(), "{name} never opened");
        assert!(
            levels.iter().all(|l| *l == Level::DEBUG),
            "{name}: {levels:?}"
        );
    }

    drop(std::fs::remove_file(hang));
}
