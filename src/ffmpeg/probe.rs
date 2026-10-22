use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::process::Command;

use super::Ffmpeg;
use crate::db::media::ProbeFields;

#[derive(Debug, Clone)]
pub struct ProbeInfo {
    /// Normalised lowercase token (`mp4`, `mov`, `m4v`, `webm`, `mkv`, `avi`, ...).
    pub container: String,
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub fps: Option<f64>,
    pub bitrate: Option<i64>,
    pub creation_time: Option<DateTime<Utc>>,
    pub raw: Value,
}

impl From<ProbeInfo> for ProbeFields {
    fn from(p: ProbeInfo) -> ProbeFields {
        ProbeFields {
            duration_ms: p.duration_ms,
            width: p.width,
            height: p.height,
            codec_video: p.vcodec,
            codec_audio: p.acodec,
            container: Some(p.container),
            fps: p.fps,
            bitrate: p.bitrate,
            probe_json: Some(p.raw.to_string()),
        }
    }
}

impl Ffmpeg {
    pub async fn probe(&self, path: &Path) -> anyhow::Result<ProbeInfo> {
        let bin = self
            .ffprobe
            .as_ref()
            .ok_or_else(|| anyhow!("ffprobe not found"))?;
        let mut cmd = Command::new(bin);
        cmd.args(["-v", "error", "-print_format", "json"])
            .args(["-show_format", "-show_streams"])
            .arg(path)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        super::no_window(&mut cmd);
        let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
            .await
            .map_err(|_| anyhow!("ffprobe timed out"))?
            .context("running ffprobe")?;
        if !out.status.success() {
            bail!(
                "ffprobe failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let raw: Value = serde_json::from_slice(&out.stdout).context("parsing ffprobe output")?;
        Ok(parse(raw, path))
    }
}

pub fn parse(raw: Value, path: &Path) -> ProbeInfo {
    let streams = raw["streams"].as_array().cloned().unwrap_or_default();
    let stream_of = |kind: &str| {
        streams
            .iter()
            .find(|s| s["codec_type"].as_str() == Some(kind))
    };
    let video = stream_of("video");
    let audio = stream_of("audio");
    let format = &raw["format"];

    let num = |v: &Value| -> Option<f64> {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    };
    let duration_ms = num(&format["duration"])
        .or_else(|| video.and_then(|v| num(&v["duration"])))
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| (d * 1000.0).round() as i64);
    let bitrate = num(&format["bit_rate"]).map(|b| b as i64);
    let creation_time = format["tags"]["creation_time"]
        .as_str()
        .or_else(|| video.and_then(|v| v["tags"]["creation_time"].as_str()))
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    ProbeInfo {
        container: normalise_container(format["format_name"].as_str().unwrap_or(""), &ext),
        duration_ms,
        width: video.and_then(|v| v["width"].as_i64()),
        height: video.and_then(|v| v["height"].as_i64()),
        vcodec: video.and_then(|v| v["codec_name"].as_str().map(str::to_string)),
        acodec: audio.and_then(|v| v["codec_name"].as_str().map(str::to_string)),
        fps: video.and_then(|v| v["r_frame_rate"].as_str().and_then(parse_rate)),
        bitrate,
        creation_time,
        raw,
    }
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d != 0.0 && n > 0.0).then(|| n / d)
}

fn normalise_container(format_name: &str, ext: &str) -> String {
    let lower = format_name.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split(',').map(str::trim).collect();
    let has = |k: &str| parts.contains(&k);
    if ["mp4", "mov", "m4a", "3gp"].iter().any(|k| has(k)) {
        return if matches!(ext, "mp4" | "m4v" | "mov") {
            ext.to_string()
        } else {
            "mp4".to_string()
        };
    }
    if has("matroska") || has("webm") {
        return if ext == "webm" { "webm" } else { "mkv" }.to_string();
    }
    if has("avi") {
        return "avi".to_string();
    }
    parts.first().map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers() {
        assert_eq!(normalise_container("mov,mp4,m4a,3gp,3g2,mj2", "mov"), "mov");
        assert_eq!(normalise_container("mov,mp4,m4a,3gp,3g2,mj2", "xyz"), "mp4");
        assert_eq!(normalise_container("matroska,webm", "webm"), "webm");
        assert_eq!(normalise_container("matroska,webm", "mkv"), "mkv");
        assert_eq!(normalise_container("avi", "avi"), "avi");
        assert_eq!(normalise_container("flv", "flv"), "flv");
    }

    #[test]
    fn rates() {
        assert!((parse_rate("60000/1001").unwrap() - 59.94).abs() < 0.01);
        assert_eq!(parse_rate("30/1"), Some(30.0));
        assert_eq!(parse_rate("0/0"), None);
    }
}
