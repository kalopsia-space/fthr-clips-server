use std::ffi::OsString;
use std::time::Duration;

use super::Ffmpeg;
use crate::config::EncoderChoice;

const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    X264,
    Nvenc,
    Vaapi,
    Qsv,
    VideoToolbox,
}

impl Encoder {
    pub fn name(&self) -> &'static str {
        match self {
            Encoder::X264 => "libx264",
            Encoder::Nvenc => "h264_nvenc",
            Encoder::Vaapi => "h264_vaapi",
            Encoder::Qsv => "h264_qsv",
            Encoder::VideoToolbox => "h264_videotoolbox",
        }
    }

    /// Output-side video/audio arguments for a transcode to `height` p.
    pub fn video_args(&self, height: u32, fps: f64) -> Vec<OsString> {
        let (maxrate, bufsize) = rate_caps(height, fps);
        let h = height.to_string();
        let mut a: Vec<String> = Vec::new();
        let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
        match self {
            Encoder::X264 => push(&["-c:v", "libx264", "-preset", "fast", "-crf", "23"]),
            Encoder::Nvenc => push(&["-c:v", "h264_nvenc", "-preset", "p4", "-cq", "23"]),
            Encoder::Qsv => push(&["-c:v", "h264_qsv", "-global_quality", "23"]),
            Encoder::VideoToolbox => push(&["-c:v", "h264_videotoolbox", "-q:v", "60"]),
            Encoder::Vaapi => {
                push(&["-vaapi_device", VAAPI_DEVICE]);
                push(&["-vf", &format!("format=nv12,hwupload,scale_vaapi=-2:{h}")]);
                push(&["-c:v", "h264_vaapi", "-qp", "23"]);
            }
        }
        if *self != Encoder::Vaapi {
            push(&["-vf", &format!("scale=-2:{h}")]);
        }
        push(&["-maxrate", maxrate, "-bufsize", bufsize]);
        push(&["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"]);
        if *self != Encoder::Vaapi {
            push(&["-pix_fmt", "yuv420p"]);
        }
        a.into_iter().map(OsString::from).collect()
    }
}

/// `(maxrate, bufsize)` for a target height; bufsize is twice maxrate.
fn rate_caps(height: u32, fps: f64) -> (&'static str, &'static str) {
    let high_fps = fps > 50.0;
    match height {
        0..=480 => ("2.5M", "5M"),
        481..=720 if high_fps => ("8M", "16M"),
        481..=720 => ("5M", "10M"),
        _ if high_fps => ("12M", "24M"),
        _ => ("8M", "16M"),
    }
}

/// Arguments for a tiny 1 s test encode: the real encoder args without the scale step.
fn probe_args(e: Encoder) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=1:size=256x144:rate=30",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let full = e.video_args(144, 30.0);
    let mut it = full.into_iter();
    while let Some(a) = it.next() {
        if a == "-vf" {
            let v = it.next().unwrap_or_default();
            if e == Encoder::Vaapi {
                args.push(a);
                args.push("format=nv12,hwupload".into());
            }
            drop(v);
            continue;
        }
        args.push(a);
    }
    args.extend(["-f", "null", "-"].iter().map(OsString::from));
    args
}

/// Find a working H.264 encoder. `None` when ffmpeg is missing or no candidate works.
pub async fn detect(ffmpeg: &Ffmpeg, choice: EncoderChoice) -> Option<Encoder> {
    if !ffmpeg.available() {
        return None;
    }
    let candidates: Vec<Encoder> = match choice {
        EncoderChoice::Auto => vec![
            Encoder::Nvenc,
            Encoder::Qsv,
            Encoder::Vaapi,
            Encoder::VideoToolbox,
            Encoder::X264,
        ],
        EncoderChoice::X264 => vec![Encoder::X264],
        EncoderChoice::Nvenc => vec![Encoder::Nvenc],
        EncoderChoice::Vaapi => vec![Encoder::Vaapi],
        EncoderChoice::Qsv => vec![Encoder::Qsv],
        EncoderChoice::VideoToolbox => vec![Encoder::VideoToolbox],
    };
    for e in candidates {
        if e == Encoder::VideoToolbox && !cfg!(target_os = "macos") {
            continue;
        }
        match ffmpeg
            .run(&probe_args(e), None, Duration::from_secs(20), None, |_| {})
            .await
        {
            Ok(()) => return Some(e),
            Err(err) => tracing::debug!("encoder {} unavailable: {err}", e.name()),
        }
    }
    None
}
