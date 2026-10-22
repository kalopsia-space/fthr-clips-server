mod common;

use std::ffi::OsString;
use std::time::Duration;

use fthr_clips_server::config::{Config, EncoderChoice};
use fthr_clips_server::ffmpeg::encoders::{self, Encoder};
use fthr_clips_server::ffmpeg::Ffmpeg;

fn config(env: &[(&str, &str)]) -> Config {
    let mut vars: std::collections::HashMap<String, String> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    vars.insert("DATA_DIR".into(), "/tmp/fthr-ffmpeg-test".into());
    Config::from_env_map(&vars, None).unwrap()
}

fn located() -> Option<Ffmpeg> {
    let f = Ffmpeg::locate(&config(&[]));
    if f.available() {
        Some(f)
    } else {
        eprintln!("skipped: ffmpeg not found");
        None
    }
}

#[tokio::test]
async fn probe_reports_duration_and_codecs() {
    let Some(f) = located() else { return };
    let Some(path) = common::fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let p = f.probe(&path).await.unwrap();
    let d = p.duration_ms.unwrap();
    assert!((1800..=2300).contains(&d), "duration {d}");
    assert_eq!(p.vcodec.as_deref(), Some("h264"));
    assert_eq!(p.acodec.as_deref(), Some("aac"));
    assert_eq!(p.width, Some(320));
    assert_eq!(p.height, Some(240));
    assert!((p.fps.unwrap() - 30.0).abs() < 0.01);
}

#[tokio::test]
async fn probe_container_normalisation() {
    let Some(f) = located() else { return };
    let Some(path) = common::fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let p = f.probe(&path).await.unwrap();
    assert_eq!(p.container, "mp4");
}

#[tokio::test]
async fn run_reports_progress() {
    let Some(f) = located() else { return };
    let Some(path) = common::fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let mut seen: Vec<f64> = Vec::new();
    let args: Vec<OsString> = vec![
        "-i".into(),
        path.into_os_string(),
        "-c:v".into(),
        "libx264".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ];
    f.run(
        &args,
        Some(Duration::from_secs(2)),
        Duration::from_secs(60),
        None,
        |p| seen.push(p),
    )
    .await
    .unwrap();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|p| (0.0..=1.0).contains(p)));
}

#[tokio::test]
async fn detect_x264_succeeds() {
    let Some(f) = located() else { return };
    let e = encoders::detect(&f, EncoderChoice::X264).await;
    if e.is_none() {
        eprintln!("skipped: ffmpeg lacks libx264");
        return;
    }
    assert_eq!(e, Some(Encoder::X264));
}

#[tokio::test]
async fn locate_without_binaries_is_not_an_error() {
    let f = Ffmpeg::locate(&config(&[
        ("FFMPEG_PATH", "/nonexistent"),
        ("FFPROBE_PATH", "/nonexistent"),
    ]));
    assert!(!f.available());
    assert!(f.probe(std::path::Path::new("x.mp4")).await.is_err());
    assert_eq!(encoders::detect(&f, EncoderChoice::Auto).await, None);
}

#[test]
fn video_args_match_spec() {
    let s = |v: Vec<OsString>| {
        v.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let x = s(Encoder::X264.video_args(720, 60.0));
    assert!(x.contains("-maxrate 8M"), "{x}");
    assert!(x.contains("-c:v libx264"), "{x}");
    assert!(x.contains("-bufsize 16M"), "{x}");
    assert!(s(Encoder::Nvenc.video_args(1080, 30.0)).contains("h264_nvenc"));
    assert!(s(Encoder::Nvenc.video_args(1080, 30.0)).contains("-maxrate 8M"));
    let v = s(Encoder::Vaapi.video_args(480, 30.0));
    assert!(v.contains("scale_vaapi=-2:480"), "{v}");
    assert!(!v.contains("-pix_fmt"), "{v}");
}
