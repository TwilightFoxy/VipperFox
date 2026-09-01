use crate::models::{
    AuthSession, ConnectRequest, HelixPage, HelixStream, HelixUser, HelixVip, ValidateResponse,
    VipUser,
};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde_json::{json, Value};
use std::{error::Error as StdError, time::Duration};
use tokio::time::sleep;

const ID_BASE: &str = "https://id.twitch.tv/oauth2";
const API_BASE: &str = "https://api.twitch.tv/helix";

#[derive(Clone)]
pub struct TwitchClient {
    http: Client,
}

impl TwitchClient {
    pub fn new() -> Result<Self, String> {
        let http = Client::builder()
            .user_agent(concat!("VipperFox/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(12))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { http })
    }

    pub async fn validate(&self, request: &ConnectRequest) -> Result<AuthSession, String> {
        let response = self
            .send_with_retry(
                self.http
                    .get(format!("{ID_BASE}/validate"))
                    .header("Authorization", format!("OAuth {}", request.access_token)),
                "проверка токена",
            )
            .await?;
        if !response.status().is_success() {
            return Err("Access Token недействителен или истёк".into());
        }
        let validation: ValidateResponse = response
            .json()
            .await
            .map_err(|error| format!("Некорректный ответ Twitch: {error}"))?;
        if validation.client_id != request.client_id {
            return Err("Client ID не соответствует Access Token".into());
        }
        let missing: Vec<&str> = ["channel:manage:vips", "user:read:chat"]
            .into_iter()
            .filter(|scope| !validation.scopes.iter().any(|value| value == scope))
            .collect();
        if !missing.is_empty() {
            return Err(format!("В токене отсутствуют права: {}", missing.join(", ")));
        }

        let temporary = AuthSession {
            client_id: request.client_id.clone(),
            access_token: request.access_token.clone(),
            broadcaster_id: validation.user_id,
            login: validation.login,
            display_name: String::new(),
        };
        let user_page: HelixPage<HelixUser> = self
            .request_json(Method::GET, "/users", &temporary, None, None)
            .await?;
        let user = user_page
            .data
            .into_iter()
            .next()
            .ok_or("Twitch не вернул данные владельца канала")?;
        if user.id != temporary.broadcaster_id {
            return Err("Токен не принадлежит найденному владельцу канала".into());
        }
        Ok(AuthSession {
            display_name: user.display_name,
            login: user.login,
            ..temporary
        })
    }

    async fn request_json<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        auth: &AuthSession,
        query: Option<&[(&str, &str)]>,
        body: Option<Value>,
    ) -> Result<T, String> {
        let mut request = self
            .http
            .request(method, format!("{API_BASE}{path}"))
            .header("Authorization", format!("Bearer {}", auth.access_token))
            .header("Client-Id", &auth.client_id);
        if let Some(query) = query {
            request = request.query(query);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = self.send_with_retry(request, path).await?;
        let status = response.status();
        if !status.is_success() {
            let message = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| "Twitch API отклонил запрос".to_owned());
            return Err(format!("{message} (HTTP {})", status.as_u16()));
        }
        response
            .json()
            .await
            .map_err(|error| format!("Не удалось разобрать ответ Twitch: {error}"))
    }

    async fn send_with_retry(
        &self,
        request: RequestBuilder,
        operation: &str,
    ) -> Result<Response, String> {
        let first = request
            .try_clone()
            .ok_or_else(|| "Не удалось подготовить запрос Twitch".to_owned())?
            .send()
            .await;
        match first {
            Ok(response) => Ok(response),
            Err(first_error) => {
                sleep(Duration::from_millis(650)).await;
                request
                    .send()
                    .await
                    .map_err(|second_error| network_error(operation, &second_error, &first_error))
            }
        }
    }

    pub async fn get_vips(&self, auth: &AuthSession) -> Result<Vec<VipUser>, String> {
        let mut result = vec![];
        let mut cursor: Option<String> = None;
        loop {
            let mut query = vec![
                ("broadcaster_id", auth.broadcaster_id.as_str()),
                ("first", "100"),
            ];
            if let Some(value) = cursor.as_deref() {
                query.push(("after", value));
            }
            let page: HelixPage<HelixVip> = self
                .request_json(Method::GET, "/channels/vips", auth, Some(&query), None)
                .await?;
            result.extend(page.data.into_iter().map(|vip| VipUser {
                user_id: vip.user_id,
                login: vip.user_login,
                display_name: vip.user_name,
                watch_streak: None,
                wrote_this_stream: false,
            }));
            cursor = page.pagination.cursor;
            if cursor.is_none() {
                result.sort_by_key(|vip| vip.login.to_lowercase());
                return Ok(result);
            }
        }
    }

    pub async fn live_stream(&self, auth: &AuthSession) -> Result<Option<HelixStream>, String> {
        let query = [("user_id", auth.broadcaster_id.as_str())];
        let page: HelixPage<HelixStream> = self
            .request_json(Method::GET, "/streams", auth, Some(&query), None)
            .await?;
        Ok(page.data.into_iter().next())
    }

    pub async fn add_vip(&self, auth: &AuthSession, user_id: &str) -> Result<(), String> {
        let query = [
            ("broadcaster_id", auth.broadcaster_id.as_str()),
            ("user_id", user_id),
        ];
        let response = self
            .http
            .post(format!("{API_BASE}/channels/vips"))
            .header("Authorization", format!("Bearer {}", auth.access_token))
            .header("Client-Id", &auth.client_id)
            .query(&query)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::CONFLICT => Err("На канале нет свободных VIP-слотов".into()),
            StatusCode::UNPROCESSABLE_ENTITY => Err("Пользователь уже VIP или является модератором".into()),
            status => Err(format!("Twitch не выдал VIP (HTTP {})", status.as_u16())),
        }
    }

    pub async fn remove_vip(&self, auth: &AuthSession, user_id: &str) -> Result<(), String> {
        let query = [
            ("broadcaster_id", auth.broadcaster_id.as_str()),
            ("user_id", user_id),
        ];
        let response = self
            .http
            .delete(format!("{API_BASE}/channels/vips"))
            .header("Authorization", format!("Bearer {}", auth.access_token))
            .header("Client-Id", &auth.client_id)
            .query(&query)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::UNPROCESSABLE_ENTITY => Ok(()),
            status => Err(format!("Twitch не снял VIP (HTTP {})", status.as_u16())),
        }
    }

    pub async fn subscribe(
        &self,
        auth: &AuthSession,
        session_id: &str,
        event_type: &str,
        chat_event: bool,
    ) -> Result<(), String> {
        let condition = if chat_event {
            json!({
                "broadcaster_user_id": auth.broadcaster_id,
                "user_id": auth.broadcaster_id
            })
        } else {
            json!({ "broadcaster_user_id": auth.broadcaster_id })
        };
        let response = self
            .http
            .post(format!("{API_BASE}/eventsub/subscriptions"))
            .header("Authorization", format!("Bearer {}", auth.access_token))
            .header("Client-Id", &auth.client_id)
            .json(&json!({
                "type": event_type,
                "version": "1",
                "condition": condition,
                "transport": {"method": "websocket", "session_id": session_id}
            }))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if response.status().is_success() || response.status() == StatusCode::CONFLICT {
            Ok(())
        } else {
            let status = response.status();
            let message = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| "Не удалось создать EventSub-подписку".into());
            Err(format!("{event_type}: {message} (HTTP {})", status.as_u16()))
        }
    }
}

fn network_error(operation: &str, error: &reqwest::Error, first: &reqwest::Error) -> String {
    let category = if error.is_timeout() {
        "тайм-аут"
    } else if error.is_connect() {
        "не удалось установить соединение"
    } else if error.is_request() {
        "ошибка запроса"
    } else {
        "сетевая ошибка"
    };
    let detail = error
        .source()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string());
    format!(
        "Twitch недоступен ({operation}: {category}) после двух попыток: {detail}. Первая попытка: {first}"
    )
}
