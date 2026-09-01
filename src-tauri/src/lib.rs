mod eventsub;
mod models;
mod state;
mod storage;
mod twitch;

use models::{ActivityKind, AppSnapshot, ConnectRequest};
use state::AppState;
use std::{fs, time::Duration};
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
    state.storage.set_setting("client_id", &resolved_request.client_id)?;
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
    state.storage.set_setting("streak_threshold", &value.to_string())?;
    state.core.write().await.snapshot.streak_threshold = value;
    state.emit_snapshot(&app).await;
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn refresh_vips(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let auth = state.auth().await.ok_or("Twitch не подключён")?;
    let mut vips = state.twitch.get_vips(&auth).await?;
    {
        let mut core = state.core.write().await;
        for vip in &mut vips {
            if let Some(previous) = core
                .snapshot
                .vips
                .iter()
                .find(|item| item.user_id == vip.user_id)
            {
                vip.watch_streak = previous.watch_streak;
                vip.wrote_this_stream = previous.wrote_this_stream;
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
    user_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    if user_ids.len() > 200 {
        return Err("Слишком много пользователей в одной операции".into());
    }
    let auth = state.auth().await.ok_or("Twitch не подключён")?;
    for (index, user_id) in user_ids.iter().enumerate() {
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
        state.emit_snapshot(&app).await;
        // Twitch permits at most 10 removals per 10 seconds. A steady 1.1s cadence
        // stays below the broadcaster-specific limit and gives the UI live progress.
        if index + 1 < user_ids.len() {
            tokio::time::sleep(Duration::from_millis(1_100)).await;
        }
    }
    Ok(state.snapshot().await)
}

async fn restore_session(app: AppHandle, state: AppState) {
    let Ok(Some(client_id)) = state.storage.setting("client_id") else {
        return;
    };
    let Ok(Some(access_token)) = state.storage.load_token() else {
        return;
    };
    let request = ConnectRequest { client_id, access_token };
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
                .push_activity(
                    ActivityKind::Warning,
                    "Нужно переподключить Twitch",
                    &error,
                )
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
