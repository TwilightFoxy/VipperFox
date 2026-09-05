use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VipUser {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub watch_streak: Option<u64>,
    pub wrote_this_stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub id: String,
    pub started_at: String,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReportUser {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub watch_streak: Option<u64>,
    pub wrote_this_stream: bool,
    pub selected: bool,
}

impl From<&VipUser> for ReportUser {
    fn from(value: &VipUser) -> Self {
        Self {
            user_id: value.user_id.clone(),
            login: value.login.clone(),
            display_name: value.display_name.clone(),
            watch_streak: value.watch_streak,
            wrote_this_stream: value.wrote_this_stream,
            selected: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    VipAdd,
    VipRemove,
    System,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub id: i64,
    pub kind: ActivityKind,
    pub title: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub configured: bool,
    pub connection_status: ConnectionStatus,
    pub channel_login: Option<String>,
    pub channel_display_name: Option<String>,
    pub stream: Option<StreamInfo>,
    pub vip_count: usize,
    pub vips: Vec<VipUser>,
    pub report: Vec<ReportUser>,
    pub report_complete: bool,
    pub activities: Vec<ActivityEntry>,
    pub streak_threshold: u64,
    pub last_error: Option<String>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            configured: false,
            connection_status: ConnectionStatus::Disconnected,
            channel_login: None,
            channel_display_name: None,
            stream: None,
            vip_count: 0,
            vips: vec![],
            report: vec![],
            report_complete: true,
            activities: vec![],
            streak_threshold: 150,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub client_id: String,
    pub access_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveVipsRequest {
    pub user_ids: Vec<String>,
    /// Deliberately typed by the operator in the confirmation dialog.
    pub channel_confirmation: String,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub client_id: String,
    pub access_token: String,
    pub broadcaster_id: String,
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct ValidateResponse {
    pub client_id: String,
    pub login: String,
    pub user_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct HelixPage<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub pagination: Pagination,
}

#[derive(Debug, Default, Deserialize)]
pub struct Pagination {
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HelixUser {
    pub id: String,
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct HelixVip {
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
}

#[derive(Debug, Deserialize)]
pub struct HelixStream {
    pub id: String,
    pub started_at: String,
}
