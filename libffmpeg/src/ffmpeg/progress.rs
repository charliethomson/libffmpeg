use std::time::Duration;

use serde::{Deserialize, Serialize};

pub enum PartialProgressState {
    Continue,
    End,
    Unknown(String),
    Unset,
}
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

    pub fn finish(&self) -> Option<Progress> {
        let progress = match &self.progress {
            PartialProgressState::Unset => return None,
            PartialProgressState::Continue => ProgressState::Continue,
            PartialProgressState::End => ProgressState::End,
            PartialProgressState::Unknown(v) => ProgressState::Unknown(v.clone()),
        };

        let num_part = self.bitrate.split("kbits").next().unwrap_or("0");
        let kbitsf = num_part.parse::<f32>().ok()?;
        let bitrate = (kbitsf * 1024.0) as isize;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub frame: usize,
    pub fps: f64,
    pub bitrate: isize,
    pub total_size: usize,
    pub out_time: Duration,
    pub dup_frames: usize,
    pub drop_frames: usize,
    pub speed: f64,
    pub progress: ProgressState,
}
