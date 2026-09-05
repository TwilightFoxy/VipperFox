use crate::{
    models::{ActivityKind, AppSnapshot, AuthSession, ConnectionStatus},
    storage::Storage,
    twitch::TwitchClient,
};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

pub struct CoreState {
    pub snapshot: AppSnapshot,
    pub auth: Option<AuthSession>,
    pub chatters: HashSet<String>,
    pub chatters_overflowed: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<RwLock<CoreState>>,
    pub twitch: TwitchClient,
    pub storage: Arc<Storage>,
    generation: Arc<AtomicU64>,
    removal_in_progress: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(storage: Storage) -> Result<Self, String> {
        let threshold = storage
            .setting("streak_threshold")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(150);
        let snapshot = AppSnapshot {
            streak_threshold: threshold,
            activities: storage.activities(100)?,
            ..AppSnapshot::default()
        };
        Ok(Self {
            core: Arc::new(RwLock::new(CoreState {
                snapshot,
                auth: None,
                chatters: HashSet::new(),
                chatters_overflowed: false,
            })),
            twitch: TwitchClient::new()?,
            storage: Arc::new(storage),
            generation: Arc::new(AtomicU64::new(0)),
            removal_in_progress: Arc::new(AtomicBool::new(false)),
        })
    }

    pub async fn snapshot(&self) -> AppSnapshot {
        self.core.read().await.snapshot.clone()
    }

    pub async fn emit_snapshot(&self, app: &AppHandle) {
        let _ = app.emit("snapshot-updated", self.snapshot().await);
    }

    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn generation_is(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    pub fn begin_removal(&self) -> bool {
        self.removal_in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn finish_removal(&self) {
        self.removal_in_progress.store(false, Ordering::SeqCst);
    }

    pub async fn auth(&self) -> Option<AuthSession> {
        self.core.read().await.auth.clone()
    }

    pub async fn configure(&self, auth: AuthSession) {
        let mut core = self.core.write().await;
        core.snapshot.configured = true;
        core.snapshot.channel_login = Some(auth.login.clone());
        core.snapshot.channel_display_name = Some(auth.display_name.clone());
        core.snapshot.connection_status = ConnectionStatus::Connecting;
        core.snapshot.last_error = None;
        core.auth = Some(auth);
    }

    pub async fn push_activity(&self, kind: ActivityKind, title: &str, detail: &str) {
        if let Ok(activity) = self.storage.add_activity(kind, title, detail) {
            let mut core = self.core.write().await;
            core.snapshot.activities.insert(0, activity);
            core.snapshot.activities.truncate(100);
        }
    }

    pub async fn mark_connection_error(&self, message: String) {
        let mut core = self.core.write().await;
        core.snapshot.connection_status = ConnectionStatus::Error;
        core.snapshot.last_error = Some(message);
        if let Some(stream) = core.snapshot.stream.as_mut() {
            stream.complete = false;
        }
        // Once EventSub is unavailable we cannot prove that a VIP from an old
        // report was not removed and re-added manually. Fail closed: keep the
        // report visible, but make every removal from it impossible.
        if !core.snapshot.report.is_empty() {
            core.snapshot.report_complete = false;
        }
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        self.next_generation();
        self.storage.delete_token()?;
        self.storage.delete_setting("client_id")?;
        let threshold = self.core.read().await.snapshot.streak_threshold;
        let activities = self.storage.activities(100)?;
        let mut core = self.core.write().await;
        core.auth = None;
        core.chatters.clear();
        core.chatters_overflowed = false;
        core.snapshot = AppSnapshot::default();
        core.snapshot.streak_threshold = threshold;
        core.snapshot.activities = activities;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReportUser;

    #[tokio::test]
    async fn connection_loss_invalidates_an_existing_report() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("state.db")).unwrap();
        let state = AppState::new(storage).unwrap();
        {
            let mut core = state.core.write().await;
            core.snapshot.report.push(ReportUser {
                user_id: "42".into(),
                login: "viewer".into(),
                display_name: "Viewer".into(),
                watch_streak: None,
                wrote_this_stream: false,
                selected: false,
            });
            core.snapshot.report_complete = true;
        }

        state.mark_connection_error("offline".into()).await;

        assert!(!state.snapshot().await.report_complete);
    }
}
