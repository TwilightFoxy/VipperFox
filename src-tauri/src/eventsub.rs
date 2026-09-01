use crate::{
    models::{ActivityKind, ConnectionStatus, ReportUser, StreamInfo, VipUser},
    state::AppState,
};
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use tauri::AppHandle;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";

enum ConnectionExit {
    Reconnect(String),
    Closed,
}

pub async fn run(app: AppHandle, state: AppState, generation: u64) {
    let mut delay = 1_u64;
    let mut url = EVENTSUB_URL.to_owned();
    let mut transferred = false;
    let mut processed = HashSet::new();
    let mut processed_order = VecDeque::new();

    while state.generation_is(generation) {
        match connection(
            &app,
            &state,
            generation,
            &url,
            transferred,
            &mut processed,
            &mut processed_order,
        )
        .await
        {
            Ok(ConnectionExit::Reconnect(next)) => {
                url = next;
                transferred = true;
                delay = 1;
            }
            Ok(ConnectionExit::Closed) if state.generation_is(generation) => {
                state.mark_connection_error("Соединение EventSub прервано".to_owned()).await;
                state.emit_snapshot(&app).await;
                sleep(Duration::from_secs(delay)).await;
                delay = (delay * 2).min(60);
                url = EVENTSUB_URL.to_owned();
                transferred = false;
            }
            Err(error) if state.generation_is(generation) => {
                state.mark_connection_error(error).await;
                state.emit_snapshot(&app).await;
                sleep(Duration::from_secs(delay)).await;
                delay = (delay * 2).min(60);
                url = EVENTSUB_URL.to_owned();
                transferred = false;
            }
            _ => break,
        }
    }
}

async fn connection(
    app: &AppHandle,
    state: &AppState,
    generation: u64,
    url: &str,
    transferred: bool,
    processed: &mut HashSet<String>,
    processed_order: &mut VecDeque<String>,
) -> Result<ConnectionExit, String> {
    if url.is_empty() {
        return Err("empty reconnect URL".into());
    }
    let auth = state.auth().await.ok_or("Twitch не подключён")?;
    let (socket, _) = connect_async(url)
        .await
        .map_err(|error| format!("EventSub WebSocket: {error}"))?;
    // Keep the write half alive for the entire EventSub session. Dropping it here
    // would close the WebSocket before notifications can arrive.
    let (_outgoing, mut incoming) = socket.split();
    let welcome = incoming
        .next()
        .await
        .ok_or("EventSub закрыл соединение до приветствия")?
        .map_err(|error| error.to_string())?;
    let welcome: Value = serde_json::from_str(
        welcome.to_text().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let session_id = welcome
        .pointer("/payload/session/id")
        .and_then(Value::as_str)
        .ok_or("В EventSub welcome отсутствует session ID")?;

    if !transferred {
        for (event_type, chat_event) in [
            ("stream.online", false),
            ("stream.offline", false),
            ("channel.chat.message", true),
            ("channel.chat.notification", true),
            ("channel.vip.add", false),
            ("channel.vip.remove", false),
        ] {
            state
                .twitch
                .subscribe(&auth, session_id, event_type, chat_event)
                .await?;
        }
        reconcile(state, &auth).await?;
    }
    {
        let mut core = state.core.write().await;
        core.snapshot.connection_status = ConnectionStatus::Connected;
        core.snapshot.last_error = None;
    }
    state.emit_snapshot(app).await;

    while let Some(message) = incoming.next().await {
        if !state.generation_is(generation) {
            return Ok(ConnectionExit::Closed);
        }
        let message = message.map_err(|error| error.to_string())?;
        if !message.is_text() {
            continue;
        }
        let payload: Value = serde_json::from_str(message.to_text().unwrap_or_default())
            .map_err(|error| error.to_string())?;
        let message_type = payload
            .pointer("/metadata/message_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if message_type == "session_reconnect" {
            let next = payload
                .pointer("/payload/session/reconnect_url")
                .and_then(Value::as_str)
                .ok_or("EventSub reconnect без URL")?;
            return Ok(ConnectionExit::Reconnect(next.to_owned()));
        }
        if message_type != "notification" {
            continue;
        }
        if let Some(message_id) = payload
            .pointer("/metadata/message_id")
            .and_then(Value::as_str)
        {
            if !processed.insert(message_id.to_owned()) {
                continue;
            }
            processed_order.push_back(message_id.to_owned());
            if processed_order.len() > 5_000 {
                if let Some(oldest) = processed_order.pop_front() {
                    processed.remove(&oldest);
                }
            }
        }
        let event_type = payload
            .pointer("/payload/subscription/type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let event = payload.pointer("/payload/event").cloned().unwrap_or(Value::Null);
        handle_event(app, state, &auth, event_type, &event).await?;
    }
    Ok(ConnectionExit::Closed)
}

async fn reconcile(state: &AppState, auth: &crate::models::AuthSession) -> Result<(), String> {
    let mut vips = state.twitch.get_vips(auth).await?;
    let live = state.twitch.live_stream(auth).await?;
    let mut core = state.core.write().await;
    for vip in &mut vips {
        vip.wrote_this_stream = core.chatters.contains(&vip.user_id);
    }
    core.snapshot.vip_count = vips.len();
    core.snapshot.vips = vips;
    if let Some(live) = live {
        match core.snapshot.stream.as_mut() {
            Some(stream) if stream.id == live.id => {}
            _ => {
                core.chatters.clear();
                core.snapshot.stream = Some(StreamInfo {
                    id: live.id,
                    started_at: live.started_at,
                    complete: false,
                });
            }
        }
    } else if core.snapshot.stream.is_some() {
        // The app may have missed stream.offline during a disconnect. Preserve a
        // useful report, but mark it incomplete because chat messages were missed.
        core.snapshot.report = core
            .snapshot
            .vips
            .iter()
            .filter(|vip| !vip.wrote_this_stream)
            .map(ReportUser::from)
            .collect();
        core.snapshot.report_complete = false;
        core.snapshot.stream = None;
        core.chatters.clear();
    }
    Ok(())
}

async fn handle_event(
    app: &AppHandle,
    state: &AppState,
    auth: &crate::models::AuthSession,
    event_type: &str,
    event: &Value,
) -> Result<(), String> {
    match event_type {
        "stream.online" => {
            let id = text(event, "id")?;
            let started_at = text(event, "started_at")?;
            let mut core = state.core.write().await;
            core.chatters.clear();
            core.snapshot.report.clear();
            core.snapshot.stream = Some(StreamInfo { id, started_at, complete: true });
            for vip in &mut core.snapshot.vips {
                vip.wrote_this_stream = false;
            }
            drop(core);
            state.push_activity(ActivityKind::System, "Стрим начался", "Мониторинг чата запущен без пропусков").await;
        }
        "stream.offline" => {
            let mut remote = state.twitch.get_vips(auth).await?;
            let mut core = state.core.write().await;
            let complete = core.snapshot.stream.as_ref().map(|stream| stream.complete).unwrap_or(false);
            for vip in &mut remote {
                vip.wrote_this_stream = core.chatters.contains(&vip.user_id);
                if let Some(previous) = core.snapshot.vips.iter().find(|item| item.user_id == vip.user_id) {
                    vip.watch_streak = previous.watch_streak;
                }
            }
            core.snapshot.report = remote.iter().filter(|vip| !vip.wrote_this_stream).map(ReportUser::from).collect();
            core.snapshot.report_complete = complete;
            core.snapshot.vip_count = remote.len();
            core.snapshot.vips = remote;
            core.snapshot.stream = None;
            core.chatters.clear();
            drop(core);
            state.push_activity(ActivityKind::System, "Стрим завершён", "Отчёт неактивных VIP готов").await;
        }
        "channel.chat.message" => {
            let user_id = text(event, "chatter_user_id")?;
            let mut core = state.core.write().await;
            core.chatters.insert(user_id.clone());
            if let Some(vip) = core.snapshot.vips.iter_mut().find(|vip| vip.user_id == user_id) {
                vip.wrote_this_stream = true;
            }
        }
        "channel.chat.notification" if event.get("notice_type").and_then(Value::as_str) == Some("watch_streak") => {
            let count = event.pointer("/watch_streak/streak_count").and_then(Value::as_u64).unwrap_or(0);
            let user_id = text(event, "chatter_user_id")?;
            let login = text(event, "chatter_user_login")?;
            let display_name = text(event, "chatter_user_name")?;
            let (threshold, already_vip, wrote) = {
                let mut core = state.core.write().await;
                let wrote = core.chatters.contains(&user_id);
                let threshold = core.snapshot.streak_threshold;
                let existing = core.snapshot.vips.iter_mut().find(|vip| vip.user_id == user_id);
                if let Some(vip) = existing {
                    vip.watch_streak = Some(count);
                    (threshold, true, wrote)
                } else {
                    (threshold, false, wrote)
                }
            };
            if count >= threshold && !already_vip {
                match state.twitch.add_vip(auth, &user_id).await {
                    Ok(()) => {
                        let mut core = state.core.write().await;
                        core.snapshot.vips.push(VipUser { user_id, login, display_name: display_name.clone(), watch_streak: Some(count), wrote_this_stream: wrote });
                        core.snapshot.vips.sort_by_key(|vip| vip.login.to_lowercase());
                        core.snapshot.vip_count = core.snapshot.vips.len();
                        drop(core);
                        state.push_activity(ActivityKind::VipAdd, &format!("{display_name} получил VIP"), &format!("Watch Streak достиг {count}")).await;
                    }
                    Err(error) => state.push_activity(ActivityKind::Warning, "Не удалось выдать VIP", &format!("{display_name}: {error}")).await,
                }
            }
        }
        "channel.vip.add" => {
            let user_id = text(event, "user_id")?;
            let mut core = state.core.write().await;
            if !core.snapshot.vips.iter().any(|vip| vip.user_id == user_id) {
                let wrote = core.chatters.contains(&user_id);
                core.snapshot.vips.push(VipUser {
                    user_id,
                    login: text(event, "user_login")?,
                    display_name: text(event, "user_name")?,
                    watch_streak: None,
                    wrote_this_stream: wrote,
                });
                core.snapshot.vip_count = core.snapshot.vips.len();
            }
        }
        "channel.vip.remove" => {
            let user_id = text(event, "user_id")?;
            let mut core = state.core.write().await;
            core.snapshot.vips.retain(|vip| vip.user_id != user_id);
            core.snapshot.report.retain(|vip| vip.user_id != user_id);
            core.snapshot.vip_count = core.snapshot.vips.len();
        }
        _ => {}
    }
    state.emit_snapshot(app).await;
    Ok(())
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("В событии Twitch отсутствует {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_contains_only_silent_vips() {
        let vips = [
            VipUser { user_id: "1".into(), login: "active".into(), display_name: "Active".into(), watch_streak: None, wrote_this_stream: true },
            VipUser { user_id: "2".into(), login: "silent".into(), display_name: "Silent".into(), watch_streak: None, wrote_this_stream: false },
        ];
        let report: Vec<ReportUser> = vips.iter().filter(|vip| !vip.wrote_this_stream).map(ReportUser::from).collect();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].user_id, "2");
    }
}
