use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::{poster, thumb, Event, JobEvent, MAX_ATTEMPTS};
use crate::app::AppState;
use crate::db::jobs::{self, Job};
use crate::db::media;

const IDLE_SLEEP: Duration = Duration::from_millis(500);

/// Spawn `concurrency` tasks that claim and run jobs of `kinds`. With no kinds
/// or no workers nothing is spawned.
pub fn spawn(state: AppState, name: &'static str, kinds: Vec<String>, concurrency: usize) {
    if kinds.is_empty() || concurrency == 0 {
        tracing::info!("{name} workers: idle ({} kinds)", kinds.len());
        return;
    }
    for n in 0..concurrency {
        let (state, kinds) = (state.clone(), kinds.clone());
        tokio::spawn(async move {
            tracing::debug!("{name} worker {n} started");
            loop {
                let refs: Vec<&str> = kinds.iter().map(String::as_str).collect();
                match jobs::claim_next(&state.db, &refs).await {
                    Ok(Some(job)) => process(&state, job).await,
                    Ok(None) => tokio::time::sleep(IDLE_SLEEP).await,
                    Err(e) => {
                        tracing::warn!("{name} worker: claim failed: {e:#}");
                        tokio::time::sleep(IDLE_SLEEP).await;
                    }
                }
            }
        });
    }
}

pub fn emit(state: &AppState, job: &Job, st: &str, progress: f64, error: Option<String>) {
    // No subscribers is the normal case; never block or fail on it.
    let _ = state.events.send(Event::Job(JobEvent {
        job_id: job.id,
        media_id: job.media_id.clone(),
        kind: job.kind.clone(),
        state: st.to_string(),
        progress,
        error,
    }));
}

async fn run_job(state: &AppState, job: &Job, cancel: CancellationToken) -> anyhow::Result<()> {
    match job.kind.as_str() {
        super::PROBE => poster::probe(state, job).await,
        super::POSTER => poster::poster(state, job, cancel).await,
        super::PREVIEW => poster::preview(state, job, cancel).await,
        super::THUMB => thumb::thumb(state, job).await,
        k if k.starts_with("transcode:") => super::transcode::transcode(state, job, cancel).await,
        other => anyhow::bail!("no handler for job kind {other}"),
    }
}

/// Removes a job's cancellation token from `AppState.running` on every path.
struct RunningGuard<'a> {
    state: &'a AppState,
    id: i64,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.state
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

async fn process(state: &AppState, job: Job) {
    let cancel = CancellationToken::new();
    state
        .running
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(job.id, cancel.clone());
    let guard = RunningGuard { state, id: job.id };
    emit(state, &job, "running", 0.0, None);
    // Own task so a panic in a handler fails the job instead of the worker.
    let handle = {
        let (state, job) = (state.clone(), job.clone());
        tokio::spawn(async move { run_job(&state, &job, cancel).await })
    };
    let result = match handle.await {
        Ok(r) => r,
        Err(join) => {
            tracing::error!(job = job.id, "job task failed: {join}");
            Err(anyhow::anyhow!("job panicked"))
        }
    };
    drop(guard);
    if let Err(e) = settle(state, &job, result).await {
        tracing::warn!(job = job.id, "recording job outcome failed: {e:#}");
    }
}

async fn settle(state: &AppState, job: &Job, result: anyhow::Result<()>) -> anyhow::Result<()> {
    let cancelled = || async {
        matches!(jobs::get(&state.db, job.id).await, Ok(Some(j)) if j.state == "cancelled")
    };
    match result {
        Ok(()) => {
            if cancelled().await {
                emit(state, job, "cancelled", 0.0, None);
                return Ok(());
            }
            if jobs::finish(&state.db, job.id).await? {
                emit(state, job, "done", 1.0, None);
            }
        }
        Err(e) => {
            if cancelled().await {
                emit(state, job, "cancelled", 0.0, None);
                return Ok(());
            }
            let msg = format!("{e:#}");
            tracing::warn!(job = job.id, kind = %job.kind, attempt = job.attempts, "job failed: {msg}");
            if job.attempts < MAX_ATTEMPTS {
                jobs::requeue(&state.db, job.id, &msg).await?;
                emit(state, job, "queued", 0.0, Some(msg));
            } else {
                jobs::fail(&state.db, job.id, &msg).await?;
                if matches!(
                    job.kind.as_str(),
                    super::PROBE | super::POSTER | super::THUMB
                ) {
                    media::set_processing(&state.db, &job.media_id, "failed", Some(&msg)).await?;
                }
                if let Some(h) = job
                    .kind
                    .strip_prefix("transcode:")
                    .and_then(|h| h.parse::<i64>().ok())
                {
                    let why = format!("gave up: {msg}");
                    jobs::rendition_fail(&state.db, &job.media_id, h, &why).await?;
                }
                emit(state, job, "failed", 0.0, Some(msg));
            }
        }
    }
    Ok(())
}
