//! `thumb` job: image thumbnails with the `image` crate (no ffmpeg needed).

use std::path::PathBuf;

use anyhow::Context;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;

use crate::app::AppState;
use crate::db::jobs::Job;
use crate::db::media;
use crate::media::paths::DerivedPaths;

const MAX_EDGE: u32 = 720;

pub async fn thumb(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let m = media::get(&state.db, &job.media_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("media {} no longer exists", job.media_id))?;
    let derived = DerivedPaths::new(&state.config.data_dir, &m.id);
    tokio::fs::create_dir_all(derived.dir()).await?;
    let (src, dest) = (PathBuf::from(&m.path), derived.thumb());
    let tmp = derived.dir().join("thumb.tmp.jpg");
    let tmp2 = tmp.clone();
    let (w, h) = tokio::task::spawn_blocking(move || -> anyhow::Result<(u32, u32)> {
        let mut reader = image::ImageReader::open(&src)
            .with_context(|| format!("opening {}", src.display()))?
            .with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        reader.limits(limits);
        let img = reader
            .decode()
            .with_context(|| format!("decoding {}", src.display()))?;
        let (w, h) = (img.width(), img.height());
        let img = if w.max(h) > MAX_EDGE {
            img.resize(MAX_EDGE, MAX_EDGE, FilterType::Lanczos3)
        } else {
            img
        };
        let file = std::fs::File::create(&tmp2)?;
        let mut out = std::io::BufWriter::new(file);
        JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img.to_rgb8())?;
        std::io::Write::flush(&mut out)?;
        Ok((w, h))
    })
    .await
    .context("thumbnail task")?
    .inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    tokio::fs::rename(&tmp, &dest).await?;
    media::set_dimensions(&state.db, &m.id, i64::from(w), i64::from(h)).await?;
    media::set_flags(&state.db, &m.id, None, None, Some(true)).await?;
    media::set_processing(&state.db, &m.id, "ready", None).await?;
    crate::webhooks::maybe_announce(state, &m.id).await;
    Ok(())
}
