//! Thin wrapper over the `ffmpeg` / `ffprobe` binaries. The server must run
//! without them, so nothing here panics or errors when they are missing.

pub mod encoders;
pub mod probe;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

const STDERR_TAIL: usize = 2048;

#[derive(Debug, Clone, Default)]
pub struct Ffmpeg {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub version: Option<String>,
    x264: std::sync::Arc<tokio::sync::OnceCell<bool>>,
}

impl Ffmpeg {
    /// Explicit config paths win; otherwise search `PATH`. Never errors.
    pub fn locate(config: &Config) -> Ffmpeg {
        let ffmpeg = resolve(config.ffmpeg_path.as_deref(), "ffmpeg");
        let ffprobe = resolve(config.ffprobe_path.as_deref(), "ffprobe");
        let version = ffmpeg.as_deref().and_then(read_version);
        Ffmpeg {
            ffmpeg,
            ffprobe,
            version,
            x264: Default::default(),
        }
    }

    /// Whether `libx264` is listed by `ffmpeg -encoders`; checked once and cached.
    pub async fn has_x264(&self) -> bool {
        *self
            .x264
            .get_or_init(|| async {
                let Some(bin) = &self.ffmpeg else {
                    return false;
                };
                let mut cmd = Command::new(bin);
                cmd.args(["-hide_banner", "-encoders"])
                    .stdin(Stdio::null())
                    .kill_on_drop(true);
                no_window(&mut cmd);
                match tokio::time::timeout(Duration::from_secs(15), cmd.output()).await {
                    Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).contains("libx264"),
                    _ => false,
                }
            })
            .await
    }

    pub fn available(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Run ffmpeg with `args`, reporting progress (0.0..=1.0) when `total` is known.
    pub async fn run(
        &self,
        args: &[OsString],
        total: Option<Duration>,
        timeout: Duration,
        cancel: Option<CancellationToken>,
        mut on_progress: impl FnMut(f64) + Send,
    ) -> anyhow::Result<()> {
        let bin = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| anyhow!("ffmpeg not found"))?;
        let mut cmd = Command::new(bin);
        cmd.args([
            "-nostdin",
            "-hide_banner",
            "-y",
            "-progress",
            "pipe:1",
            "-loglevel",
            "error",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        no_window(&mut cmd);
        let mut child = cmd.spawn().context("spawning ffmpeg")?;
        let stdout = child.stdout.take().context("ffmpeg stdout")?;
        let mut stderr = child.stderr.take().context("ffmpeg stderr")?;

        let stderr_task = tokio::spawn(async move {
            let mut tail: Vec<u8> = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > STDERR_TAIL {
                    let cut = tail.len() - STDERR_TAIL;
                    tail.drain(..cut);
                }
            }
            String::from_utf8_lossy(&tail).trim().to_string()
        });

        let total_secs = total.map(|t| t.as_secs_f64()).filter(|t| *t > 0.0);
        let cancel = cancel.unwrap_or_default();
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        let mut lines = BufReader::new(stdout).lines();
        let mut stdout_open = true;

        let status = loop {
            tokio::select! {
                _ = &mut deadline => {
                    let _ = child.kill().await;
                    stderr_task.abort();
                    bail!("ffmpeg timed out after {}s", timeout.as_secs());
                }
                _ = cancel.cancelled() => {
                    let _ = child.kill().await;
                    stderr_task.abort();
                    bail!("ffmpeg cancelled");
                }
                line = lines.next_line(), if stdout_open => match line {
                    Ok(Some(l)) => {
                        if let (Some(total), Some(us)) = (total_secs, parse_out_time_us(&l)) {
                            on_progress(((us as f64 / 1e6) / total).clamp(0.0, 1.0));
                        }
                    }
                    _ => stdout_open = false,
                },
                status = child.wait(), if !stdout_open => {
                    break status.context("waiting for ffmpeg")?;
                }
            }
        };
        let tail = stderr_task.await.unwrap_or_default();
        if !status.success() {
            bail!("ffmpeg exited with {status}: {tail}");
        }
        Ok(())
    }
}

fn parse_out_time_us(line: &str) -> Option<i64> {
    // `out_time_ms` is (confusingly) microseconds on older builds as well.
    let v = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    v.trim().parse::<i64>().ok().filter(|n| *n >= 0)
}

#[cfg(windows)]
pub(crate) fn no_window(cmd: &mut Command) {
    // CREATE_NO_WINDOW
    cmd.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
pub(crate) fn no_window(_cmd: &mut Command) {}

fn resolve(explicit: Option<&Path>, name: &str) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if is_executable(p) {
            return Some(p.to_path_buf());
        }
        tracing::warn!("{name} path {} is not an executable file", p.display());
        return None;
    }
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(&exe))
        .find(|c| is_executable(c))
}

fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn read_version(bin: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("-version").stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?;
    let rest = first.strip_prefix("ffmpeg version ")?;
    rest.split_whitespace().next().map(str::to_string)
}
