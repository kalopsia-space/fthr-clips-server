use crate::app::AppState;
use crate::db::Media;

/// Hook called after a new media row is inserted. Replaced by the real job
/// queue in a later task.
pub async fn enqueue_for_new_media(_state: &AppState, _media: &Media) {}
