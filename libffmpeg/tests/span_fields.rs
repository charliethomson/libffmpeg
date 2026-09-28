//! `ffmpeg.*` / `ffprobe.run` record `otel.status_code` on their own span only.
//!
//! When one of those spans is filtered out (they are all DEBUG), the
//! `Span::current()` inside it is the *caller's* span; recording through it
//! would mark the caller ERROR.
//!
//! One test fn: it points the tools at a broken binary through the
//! process-global override, and this file is its own test binary.

#![cfg(unix)]

use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use libffmpeg::{
    ffmpeg::{ffmpeg, ffmpeg_graceful, ffmpeg_slim},
    ffprobe::ffprobe,
    libcmd::CommandMonitor,
    tools::{Tool, set_tool_path},
};
use tokio_util::sync::CancellationToken;
use tracing::{
    Instrument, Subscriber,
    field::{Field, Visit},
    span::{Id, Record},
};
use tracing_subscriber::{
    Layer,
    filter::LevelFilter,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
};

/// Every `(span name, field name)` pair recorded after span creation.
#[derive(Clone, Default)]
struct Recorded(Arc<Mutex<Vec<(String, String)>>>);

impl Recorded {
    fn take(&self) -> Vec<(String, String)> {
        let mut all = std::mem::take(&mut *self.0.lock().unwrap());
        all.sort();
        all
    }
}

struct FieldNames(Vec<String>);

impl Visit for FieldNames {
    fn record_debug(&mut self, field: &Field, _: &dyn std::fmt::Debug) {
        self.0.push(field.name().to_string());
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Recorded {
    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let name = ctx.span(id).expect("span exists").name().to_string();
        let mut fields = FieldNames(Vec::new());
        values.record(&mut fields);
        let mut out = self.0.lock().unwrap();
        out.extend(fields.0.into_iter().map(|f| (name.clone(), f)));
    }
}

/// An executable whose interpreter doesn't exist: `exec` fails with ENOENT,
/// which libcmd reports as `BadSpawn` — a real (non-cancel) failure.
fn broken_binary() -> PathBuf {
    let path = std::env::temp_dir().join(format!("libffmpeg-broken-{}", std::process::id()));
    std::fs::write(&path, "#!/nonexistent/interpreter\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn caller() -> tracing::Span {
    tracing::warn_span!("caller", otel.status_code = tracing::field::Empty)
}

/// Runs every entry point once, each inside its own `caller` span, and
/// checks each failed.
async fn run_all() {
    let token = CancellationToken::new;
    assert!(
        ffmpeg_slim(token(), |_| {})
            .instrument(caller())
            .await
            .is_err()
    );
    let monitor = CommandMonitor::new();
    assert!(
        ffmpeg(token(), &monitor.server, |_| {})
            .instrument(caller())
            .await
            .is_err()
    );
    assert!(
        ffmpeg_graceful(token(), &monitor.client, &monitor.server, |_| {})
            .instrument(caller())
            .await
            .is_err()
    );
    assert!(ffprobe(token(), |_| {}).instrument(caller()).await.is_err());
}

#[tokio::test]
async fn status_code_lands_on_the_owning_span_only() {
    let broken = broken_binary();
    set_tool_path(Tool::Ffmpeg, Some(broken.clone()));
    set_tool_path(Tool::Ffprobe, Some(broken.clone()));

    let recorded = Recorded::default();

    // WARN: every libffmpeg span (INFO and DEBUG) is filtered out; only the
    // caller exists, and nothing may be recorded on it.
    {
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::WARN)
            .with(recorded.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        run_all().await;
    }
    assert_eq!(recorded.take(), []);

    // TRACE: each span marks itself (libcmd.run records its own fields too).
    {
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(recorded.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        run_all().await;
    }
    let status: Vec<String> = recorded
        .take()
        .into_iter()
        .filter(|(_, field)| field == "otel.status_code")
        .map(|(span, _)| span)
        .collect();
    assert_eq!(
        status,
        [
            "ffmpeg.monitored",
            "ffmpeg.run",
            "ffmpeg.slim",
            "ffprobe.run"
        ]
    );

    drop(std::fs::remove_file(broken));
}
