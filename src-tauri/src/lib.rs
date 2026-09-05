mod eventsub;
mod models;
mod state;
mod storage;
mod twitch;

use models::{ActivityKind, AppSnapshot, AuthSession, ConnectRequest, RemoveVipsRequest};
use state::AppState;
use std::{collections::HashSet, fs, time::Duration};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State,
};

#[tauri::command]
async fn get_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn connect_twitch(
    request: ConnectRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let (auth, resolved_request, fields_swapped) = match state.twitch.validate(&request).await {
        Ok(auth) => (auth, request, false),
        Err(original_error) => {
            let swapped = ConnectRequest {
                client_id: request.access_token.clone(),
                access_token: request.client_id.clone(),
            };
            match state.twitch.validate(&swapped).await {
                Ok(auth) => (auth, swapped, true),
                Err(_) => return Err(original_error),
            }
        }
    };
    state.storage.save_token(&resolved_request.access_token)?;
    if let Err(error) = state
        .storage
        .set_setting("client_id", &resolved_request.client_id)
    {
        let _ = state.storage.delete_token();
        return Err(error);
    }
    state.configure(auth).await;
    state
        .push_activity(
            ActivityKind::System,
            "Twitch подключён",
            "Токен проверен, запускаем EventSub",
        )
        .await;
    if fields_swapped {
        state
            .push_activity(
                ActivityKind::Warning,
                "Поля исправлены автоматически",
                "Client ID и Access Token были вставлены наоборот",
            )
            .await;
    }
    let generation = state.next_generation();
    let monitor_state = state.inner().clone();
    let monitor_app = app.clone();
    tauri::async_runtime::spawn(async move {
        eventsub::run(monitor_app, monitor_state, generation).await;
    });
    state.emit_snapshot(&app).await;
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn disconnect_twitch(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    state.disconnect().await?;
    state.emit_snapshot(&app).await;
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn set_streak_threshold(
    value: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let value = value.max(1);
    state
        .storage
        .set_setting("streak_threshold", &value.to_string())?;
    state.core.write().await.snapshot.streak_threshold = value;
    state.emit_snapshot(&app).await;
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn refresh_vips(app: AppHandle, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let auth = state.auth().await.ok_or("Twitch не подключён")?;
    let mut vips = state.twitch.get_vips(&auth).await?;
    {
        let mut core = state.core.write().await;
        for vip in &mut vips {
            vip.wrote_this_stream = core.chatters.contains(&vip.user_id);
            if let Some(previous) = core
                .snapshot
                .vips
                .iter()
                .find(|item| item.user_id == vip.user_id)
            {
                vip.watch_streak = previous.watch_streak;
                vip.wrote_this_stream |= previous.wrote_this_stream;
            }
        }
        core.snapshot.vip_count = vips.len();
        core.snapshot.vips = vips;
        core.snapshot.last_error = None;
    }
    state.emit_snapshot(&app).await;
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn remove_vips(
    request: RemoveVipsRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    if !state.begin_removal() {
        return Err("Снятие VIP уже выполняется. Дождитесь его завершения".into());
    }
    let result = remove_vips_inner(&request, &app, &state).await;
    state.finish_removal();
    result
}

async fn remove_vips_inner(
    request: &RemoveVipsRequest,
    app: &AppHandle,
    state: &AppState,
) -> Result<AppSnapshot, String> {
    let auth = state.auth().await.ok_or("Twitch не подключён")?;
    let snapshot = state.snapshot().await;
    validate_removal_request(&snapshot, &auth, request)?;

    // Re-read Twitch immediately before mutation. A stale report must never be
    // enough to remove a role which is no longer present in the current list.
    let current_vips: HashSet<String> = state
        .twitch
        .get_vips(&auth)
        .await?
        .into_iter()
        .map(|vip| vip.user_id)
        .collect();
    if let Some(missing) = request
        .user_ids
        .iter()
        .find(|user_id| !current_vips.contains(*user_id))
    {
        return Err(format!(
            "Операция отменена: пользователь {missing} уже отсутствует в актуальном списке VIP"
        ));
    }
    if state.twitch.live_stream(&auth).await?.is_some() {
        return Err("Снятие VIP отменено: Twitch сообщает, что канал сейчас в эфире".into());
    }

    for (index, user_id) in request.user_ids.iter().enumerate() {
        let current = state.snapshot().await;
        if current.stream.is_some() || !current.report_complete {
            return Err(format!(
                "Операция безопасно остановлена после {index} снятий: состояние трансляции изменилось"
            ));
        }
        let current_auth = state.auth().await.ok_or_else(|| {
            format!("Операция безопасно остановлена после {index} снятий: Twitch отключён")
        })?;
        if current_auth.broadcaster_id != auth.broadcaster_id {
            return Err(format!(
                "Операция безопасно остановлена после {index} снятий: подключён другой канал"
            ));
        }
        if state.twitch.live_stream(&auth).await?.is_some() {
            return Err(format!(
                "Операция безопасно остановлена после {index} снятий: Twitch сообщает о начале эфира"
            ));
        }
        state.twitch.remove_vip(&auth, user_id).await?;
        let display_name = {
            let mut core = state.core.write().await;
            let name = core
                .snapshot
                .vips
                .iter()
                .find(|vip| &vip.user_id == user_id)
                .map(|vip| vip.display_name.clone())
                .unwrap_or_else(|| user_id.clone());
            core.snapshot.vips.retain(|vip| &vip.user_id != user_id);
            core.snapshot.report.retain(|vip| &vip.user_id != user_id);
            core.snapshot.vip_count = core.snapshot.vips.len();
            name
        };
        state
            .push_activity(
                ActivityKind::VipRemove,
                &format!("{display_name} лишён VIP"),
                "Ручное действие из отчёта VipperFox",
            )
            .await;
        state.emit_snapshot(app).await;
        // Twitch permits at most 10 removals per 10 seconds. A steady 1.1s cadence
        // stays below the broadcaster-specific limit and gives the UI live progress.
        if index + 1 < request.user_ids.len() {
            tokio::time::sleep(Duration::from_millis(1_100)).await;
        }
    }
    Ok(state.snapshot().await)
}

fn validate_removal_request(
    snapshot: &AppSnapshot,
    auth: &AuthSession,
    request: &RemoveVipsRequest,
) -> Result<(), String> {
    const MAX_REMOVALS_PER_OPERATION: usize = 20;
    if request.user_ids.is_empty() {
        return Err("Не выбран ни один пользователь".into());
    }
    if request.user_ids.len() > MAX_REMOVALS_PER_OPERATION {
        return Err(format!(
            "За одну операцию можно снять VIP максимум у {MAX_REMOVALS_PER_OPERATION} пользователей"
        ));
    }
    if snapshot.stream.is_some() {
        return Err("Снятие VIP заблокировано до завершения трансляции".into());
    }
    if !snapshot.report_complete {
        return Err(
            "Снятие VIP заблокировано: мониторинг трансляции был неполным. Проверьте пользователей вручную в Twitch"
                .into(),
        );
    }
    if !request
        .channel_confirmation
        .trim()
        .eq_ignore_ascii_case(&auth.login)
    {
        return Err("Подтверждение не совпадает с логином подключённого канала".into());
    }

    let unique: HashSet<&str> = request.user_ids.iter().map(String::as_str).collect();
    if unique.len() != request.user_ids.len() || unique.iter().any(|id| id.trim().is_empty()) {
        return Err("Список пользователей повреждён или содержит повторы".into());
    }
    let report_ids: HashSet<&str> = snapshot
        .report
        .iter()
        .map(|user| user.user_id.as_str())
        .collect();
    if !unique.is_subset(&report_ids) {
        return Err("Операция отменена: выбран пользователь не из текущего отчёта".into());
    }
    Ok(())
}

async fn restore_session(app: AppHandle, state: AppState) {
    let Ok(Some(client_id)) = state.storage.setting("client_id") else {
        return;
    };
    let Ok(Some(access_token)) = state.storage.load_token() else {
        return;
    };
    let request = ConnectRequest {
        client_id,
        access_token,
    };
    match state.twitch.validate(&request).await {
        Ok(auth) => {
            state.configure(auth).await;
            let generation = state.next_generation();
            let monitor_state = state.clone();
            let monitor_app = app.clone();
            tauri::async_runtime::spawn(async move {
                eventsub::run(monitor_app, monitor_state, generation).await;
            });
        }
        Err(error) => {
            let _ = state.storage.delete_token();
            let _ = state.storage.delete_setting("client_id");
            state
                .push_activity(ActivityKind::Warning, "Нужно переподключить Twitch", &error)
                .await;
        }
    }
    state.emit_snapshot(&app).await;
}

fn create_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Открыть VipperFox", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Завершить", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::new()
        .tooltip("VipperFox — мониторинг Twitch")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = storage::Storage::open(&data_dir.join("vipperfox.db"))
                .map_err(std::io::Error::other)?;
            let state = AppState::new(storage).map_err(std::io::Error::other)?;
            app.manage(state.clone());
            create_tray(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { restore_session(handle, state).await });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            connect_twitch,
            disconnect_twitch,
            set_streak_threshold,
            refresh_vips,
            remove_vips
        ])
        .run(tauri::generate_context!())
        .expect("failed to run VipperFox");
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    use models::{ConnectionStatus, ReportUser};

    fn fixture(complete: bool, live: bool) -> (AppSnapshot, AuthSession, RemoveVipsRequest) {
        let report_user = ReportUser {
            user_id: "42".into(),
            login: "viewer".into(),
            display_name: "Viewer".into(),
            watch_streak: None,
            wrote_this_stream: false,
            selected: false,
        };
        let snapshot = AppSnapshot {
            configured: true,
            connection_status: ConnectionStatus::Connected,
            channel_login: Some("fox_channel".into()),
            channel_display_name: Some("Fox Channel".into()),
            stream: live.then(|| models::StreamInfo {
                id: "stream".into(),
                started_at: "2026-01-01T00:00:00Z".into(),
                complete,
            }),
            vip_count: 1,
            vips: vec![],
            report: vec![report_user],
            report_complete: complete,
            activities: vec![],
            streak_threshold: 150,
            last_error: None,
        };
        let auth = AuthSession {
            client_id: "client".into(),
            access_token: "token".into(),
            broadcaster_id: "7".into(),
            login: "fox_channel".into(),
            display_name: "Fox Channel".into(),
        };
        let request = RemoveVipsRequest {
            user_ids: vec!["42".into()],
            channel_confirmation: "fox_channel".into(),
        };
        (snapshot, auth, request)
    }

    #[test]
    fn valid_completed_report_is_allowed() {
        let (snapshot, auth, request) = fixture(true, false);
        assert!(validate_removal_request(&snapshot, &auth, &request).is_ok());
    }

    #[test]
    fn incomplete_report_is_never_allowed() {
        let (snapshot, auth, request) = fixture(false, false);
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
    }

    #[test]
    fn removal_during_stream_is_never_allowed() {
        let (snapshot, auth, request) = fixture(true, true);
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
    }

    #[test]
    fn arbitrary_or_duplicate_ids_are_rejected() {
        let (snapshot, auth, mut request) = fixture(true, false);
        request.user_ids = vec!["42".into(), "42".into()];
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
        request.user_ids = vec!["not-in-report".into()];
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
    }

    #[test]
    fn empty_and_oversized_batches_are_rejected() {
        let (snapshot, auth, mut request) = fixture(true, false);
        request.user_ids.clear();
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
        request.user_ids = (0..21).map(|index| index.to_string()).collect();
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
    }

    #[test]
    fn wrong_channel_confirmation_is_rejected() {
        let (snapshot, auth, mut request) = fixture(true, false);
        request.channel_confirmation = "another_channel".into();
        assert!(validate_removal_request(&snapshot, &auth, &request).is_err());
    }
}
