/// Whether a browser can play this container/codec combination directly.
pub fn is_browser_playable(container: &str, vcodec: Option<&str>, acodec: Option<&str>) -> bool {
    matches!(container, "mp4" | "m4v" | "mov" | "webm")
        && matches!(vcodec, Some("h264" | "vp9" | "av1"))
        && matches!(acodec, None | Some("aac" | "opus" | "mp3" | "vorbis"))
}
