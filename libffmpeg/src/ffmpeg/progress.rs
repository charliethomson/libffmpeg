use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Internal state tracking for [`PartialProgress`] accumulation.
pub enum PartialProgressState {
    Continue,
    End,
    Unknown(String),
    Unset,
}

/// Accumulator for parsing ffmpeg's `-progress pipe:1` output line by line.
///
/// Feed lines via [`with_line`](Self::with_line), then call [`finish`](Self::finish)
/// once a complete progress block has been received to produce a [`Progress`] snapshot.
///
/// ```no_run
/// # use libffmpeg::ffmpeg::progress::PartialProgress;
/// let mut partial = PartialProgress::default();
/// partial.with_line("frame=120");
/// partial.with_line("fps=30.00");
/// partial.with_line("progress=continue");
/// if let Some(progress) = partial.finish() {
///     println!("frame={} fps={}", progress.frame, progress.fps);
/// }
/// ```
pub struct PartialProgress {
    frame: usize,
    fps: f64,
    bitrate: String,
    total_size: usize,
    out_time_us: u128,
    dup_frames: usize,
    drop_frames: usize,
    speed: String,
    progress: PartialProgressState,
}
impl Default for PartialProgress {
    fn default() -> Self {
        Self {
            frame: 0,
            fps: 0.0,
            bitrate: String::new(),
            total_size: 0,
            out_time_us: 0,
            dup_frames: 0,
            drop_frames: 0,
            speed: String::new(),
            progress: PartialProgressState::Unset,
        }
    }
}
impl PartialProgress {
    /// Feed a single `key=value` line from ffmpeg's progress output.
    ///
    /// Returns `true` if the line was recognized (or intentionally ignored),
    /// `false` if the key was completely unknown.
    pub fn with_line(&mut self, line: &str) -> bool {
        let mut parts = line.splitn(2, '=');
        let Some(key) = parts.next() else {
            return false;
        };
        let Some(value) = parts.next() else {
            return false;
        };

        // Invalid value
        if value == "N/A" {
            return true;
        }

        macro_rules! parse_value {
            (as $ty:ty => $ident:ident) => {{
                let Ok(v) = value.parse::<$ty>() else {
                    return true;
                };
                self.$ident = v;
            }};
        }

        match key {
            "bitrate" => self.bitrate = value.trim().to_string(),
            "speed" => self.speed = value.trim().to_string(),
            "frame" => parse_value!(as usize => frame),
            "fps" => parse_value!(as f64 => fps),
            "total_size" => parse_value!(as usize => total_size),
            "out_time_us" => parse_value!(as u128 => out_time_us),
            "dup_frames" => parse_value!(as usize => dup_frames),
            "drop_frames" => parse_value!(as usize => drop_frames),
            "progress" => {
                self.progress = match value.trim() {
                    "continue" => PartialProgressState::Continue,
                    "end" => PartialProgressState::End,
                    v => PartialProgressState::Unknown(v.to_string()),
                };
            }

            // Filter explicit ignores
            key if key.starts_with("stream_") => return true,
            "out_time" | "out_time_ms" => return true,
            _ => return false,
        }
        true
    }

    /// Attempt to produce a complete [`Progress`] snapshot from the accumulated state.
    ///
    /// Returns `None` if no `progress=` line has been received yet, or if a
    /// reported bitrate could not be parsed. A bitrate ffmpeg hasn't reported
    /// yet (`N/A`) comes back as `0`.
    #[must_use]
    pub fn finish(&self) -> Option<Progress> {
        let progress = match &self.progress {
            PartialProgressState::Unset => return None,
            PartialProgressState::Continue => ProgressState::Continue,
            PartialProgressState::End => ProgressState::End,
            PartialProgressState::Unknown(v) => ProgressState::Unknown(v.clone()),
        };

        // Empty until ffmpeg reports one (it sends `bitrate=N/A` at first).
        let bitrate = if self.bitrate.is_empty() {
            0
        } else {
            let num_part = self.bitrate.split("kbits").next().unwrap_or("0");
            let kbitsf = num_part.parse::<f32>().ok()?;
            (kbitsf * 1024.0) as isize
        };
        Some(Progress {
            frame: self.frame,
            fps: self.fps,
            bitrate,
            total_size: self.total_size,
            out_time: Duration::from_micros(self.out_time_us as u64),
            dup_frames: self.dup_frames,
            drop_frames: self.drop_frames,
            speed: self.speed.trim_end_matches('x').parse().unwrap_or_default(),
            progress,
        })
    }
}

/// The state reported in ffmpeg's `progress=` line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "raw",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProgressState {
    Continue,
    End,

    Unknown(String),
}

/// A parsed progress snapshot from ffmpeg's `-progress pipe:1` output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// Number of frames processed so far.
    pub frame: usize,
    /// Current encoding speed in frames per second.
    pub fps: f64,
    /// Current bitrate in bytes per second.
    pub bitrate: isize,
    /// Total output size in bytes.
    pub total_size: usize,
    /// Elapsed output time (position in the output stream).
    pub out_time: Duration,
    /// Number of duplicated frames.
    pub dup_frames: usize,
    /// Number of dropped frames.
    pub drop_frames: usize,
    /// Encoding speed as a multiplier of realtime (e.g. 2.0 = 2x realtime).
    pub speed: f64,
    /// Whether ffmpeg is still processing or has finished.
    pub progress: ProgressState,
}

#[cfg(test)]
mod tests {
    use super::{PartialProgress, ProgressState};
    use std::time::Duration;

    fn feed(lines: &[&str]) -> PartialProgress {
        let mut partial = PartialProgress::default();
        for line in lines {
            partial.with_line(line);
        }
        partial
    }

    #[test]
    fn full_block_parses() {
        let p = feed(&[
            "frame=120",
            "fps=29.97",
            "bitrate=1500.5kbits/s",
            "total_size=1048576",
            "out_time_us=4000000",
            "dup_frames=2",
            "drop_frames=1",
            "speed=1.5x",
            "progress=continue",
        ])
        .finish()
        .expect("a complete block");

        assert_eq!(p.frame, 120);
        assert!((p.fps - 29.97).abs() < f64::EPSILON);
        assert_eq!(p.bitrate, 1_536_512);
        assert_eq!(p.total_size, 1_048_576);
        assert_eq!(p.out_time, Duration::from_secs(4));
        assert_eq!((p.dup_frames, p.drop_frames), (2, 1));
        assert!((p.speed - 1.5).abs() < f64::EPSILON);
        assert!(matches!(p.progress, ProgressState::Continue));
    }

    #[test]
    fn nothing_until_a_progress_line() {
        assert!(feed(&["frame=1", "bitrate=1kbits/s"]).finish().is_none());
    }

    #[test]
    fn end_and_unknown_states() {
        let end = feed(&["bitrate=1kbits/s", "progress=end"])
            .finish()
            .unwrap();
        assert!(matches!(end.progress, ProgressState::End));

        let odd = feed(&["bitrate=1kbits/s", "progress=paused"])
            .finish()
            .unwrap();
        assert!(matches!(odd.progress, ProgressState::Unknown(ref v) if v == "paused"));
    }

    #[test]
    fn na_and_garbage_values_keep_the_previous_value() {
        let p = feed(&[
            "frame=10",
            "frame=N/A",
            "frame=nope",
            "bitrate=1kbits/s",
            "progress=continue",
        ])
        .finish()
        .unwrap();
        assert_eq!(p.frame, 10);
    }

    #[test]
    fn line_recognition() {
        let mut p = PartialProgress::default();
        assert!(p.with_line("frame=1"));
        assert!(p.with_line("stream_0_0_q=28.0"));
        assert!(p.with_line("out_time=00:00:01.000000"));
        assert!(!p.with_line("mystery=1"));
        assert!(!p.with_line("no equals sign"));
    }

    // ffmpeg reports `bitrate=N/A` until it has written some output, so the
    // first blocks of every run carry it.
    #[test]
    fn na_bitrate_still_yields_a_snapshot() {
        let p = feed(&["frame=1", "bitrate=N/A", "progress=continue"])
            .finish()
            .expect("a snapshot despite the unknown bitrate");
        assert_eq!(p.bitrate, 0);
    }
}
