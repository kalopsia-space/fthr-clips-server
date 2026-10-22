use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use chrono::{Datelike, Local, TimeZone, Timelike};
use fthr_clips_server::media::filename_date::recorded_at_from_name;
use fthr_clips_server::media::ids::{media_id, slug_candidates};
use fthr_clips_server::media::kind::{kind_for, MediaKind};
use fthr_clips_server::media::paths::DerivedPaths;
use fthr_clips_server::media::playable::is_browser_playable;

const MIB: u64 = 1024 * 1024;

/// Sparse file of `len` bytes with `(offset, byte)` patches.
fn sparse(path: &Path, len: u64, patches: &[(u64, u8)]) {
    let mut f = File::create(path).unwrap();
    f.set_len(len).unwrap();
    for &(off, b) in patches {
        f.seek(SeekFrom::Start(off)).unwrap();
        f.write_all(&[b]).unwrap();
    }
}

#[test]
fn media_id_differs_when_tail_differs() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    sparse(&a, 40 * MIB, &[(5, 1), (40 * MIB - 1, 1)]);
    sparse(&b, 40 * MIB, &[(5, 1), (40 * MIB - 1, 2)]);
    assert_ne!(media_id(&a).unwrap(), media_id(&b).unwrap());
}

#[test]
fn media_id_ignores_middle_of_large_file() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    sparse(&a, 40 * MIB, &[]);
    sparse(&b, 40 * MIB, &[(20 * MIB, 9)]);
    assert_eq!(media_id(&a).unwrap(), media_id(&b).unwrap());
}

#[test]
fn media_id_differs_by_size() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    sparse(&a, 100, &[]);
    sparse(&b, 101, &[]);
    assert_ne!(media_id(&a).unwrap(), media_id(&b).unwrap());
}

#[test]
fn media_id_stable_for_identical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, b"hello world").unwrap();
    std::fs::write(&b, b"hello world").unwrap();
    let id = media_id(&a).unwrap();
    assert_eq!(id, media_id(&b).unwrap());
    assert_eq!(id.len(), 32);
    assert!(id
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn media_id_small_file_content_matters() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, b"hello world").unwrap();
    std::fs::write(&b, b"hello_world").unwrap();
    assert_ne!(media_id(&a).unwrap(), media_id(&b).unwrap());
}

#[test]
fn slug_candidates_are_base62_and_11_chars() {
    let c = slug_candidates("00000000000000ff00000000000000ff");
    assert_eq!(c[0].len(), 11);
    assert!(c[0].chars().all(|ch| ch.is_ascii_alphanumeric()));
    assert_eq!(c[0], "00000000047"); // 255 = 4*62 + 7
    assert_eq!(c[0], c[1]);
    assert_eq!(c[2].len(), 22);
    assert_eq!(c[2], format!("{}{}", c[0], c[1]));
}

#[test]
fn slug_candidates_max_value() {
    let c = slug_candidates("ffffffffffffffff0000000000000000");
    assert_eq!(c[0], "LygHa16AHYF");
    assert_eq!(c[1], "00000000000");
}

#[test]
fn kind_for_rejects_unknown_and_mismatch() {
    assert!(kind_for("exe", b"MZ").is_none());
    assert!(kind_for("mp4", b"\x89PNG\r\n\x1a\n").is_none());
    assert_eq!(
        kind_for("png", b"\x89PNG\r\n\x1a\n\0\0"),
        Some(MediaKind::Image)
    );
}

#[test]
fn kind_for_videos_and_case() {
    let mp4 = b"\0\0\0\x18ftypisom\0\0\0\0isomiso2mp41";
    assert_eq!(kind_for("mp4", mp4), Some(MediaKind::Video));
    assert_eq!(kind_for("MP4", mp4), Some(MediaKind::Video));
    assert_eq!(kind_for("mkv", b"\x1aE\xdf\xa3"), Some(MediaKind::Video));
    assert!(kind_for("png", mp4).is_none());
    assert!(kind_for("mp4", &[0u8; 64]).is_none());
}

#[test]
fn playable_rules() {
    assert!(is_browser_playable("mp4", Some("h264"), Some("aac")));
    assert!(!is_browser_playable("mkv", Some("h264"), Some("aac")));
    assert!(!is_browser_playable("mp4", Some("hevc"), Some("aac")));
    assert!(is_browser_playable("webm", Some("vp9"), None));
    assert!(!is_browser_playable("mp4", Some("h264"), Some("ac3")));
    assert!(!is_browser_playable("mp4", None, Some("aac")));
}

#[test]
fn filename_dates() {
    assert!(recorded_at_from_name("Clip 2026-10-22 18-03-11.mp4").is_some());
    assert!(recorded_at_from_name("20261022_180311_valorant.mkv").is_some());
    assert!(recorded_at_from_name("ace.mp4").is_none());

    let expect = Local
        .with_ymd_and_hms(2026, 10, 22, 18, 3, 11)
        .unwrap()
        .to_utc();
    assert_eq!(
        recorded_at_from_name("Clip 2026-10-22 18-03-11.mp4"),
        Some(expect)
    );
    assert_eq!(
        recorded_at_from_name("a_2026-10-22_18-03-11.mp4"),
        Some(expect)
    );
    assert_eq!(recorded_at_from_name("20261022_180311_x.mkv"), Some(expect));

    let d = recorded_at_from_name("holiday 2026-10-22.png")
        .unwrap()
        .with_timezone(&Local);
    assert_eq!((d.year(), d.month(), d.day(), d.hour()), (2026, 10, 22, 0));

    assert!(recorded_at_from_name("2026-13-45 99-99-99.mp4").is_none());
}

#[test]
fn derived_paths() {
    let dir = tempfile::tempdir().unwrap();
    let p = DerivedPaths::new(dir.path(), "abc");
    assert_eq!(p.dir(), dir.path().join("derived").join("abc"));
    assert_eq!(p.poster().file_name().unwrap(), "poster.jpg");
    assert_eq!(p.preview().file_name().unwrap(), "preview.mp4");
    assert_eq!(p.thumb().file_name().unwrap(), "thumb.jpg");
    assert_eq!(p.rendition(720).file_name().unwrap(), "720p.mp4");
    p.remove_all().unwrap(); // no-op when missing
    std::fs::create_dir_all(p.dir()).unwrap();
    std::fs::write(p.poster(), b"x").unwrap();
    p.remove_all().unwrap();
    assert!(!p.dir().exists());
}
