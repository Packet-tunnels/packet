use crate::ClientConfig;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatsSnapshot {
    pub state: String,
    pub transport: String,
    pub server_host: String,
    pub cdn_edge: Option<String>,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub active_streams: u32,
    pub total_streams: u64,
    pub connected_since: Option<u64>,
    pub last_ping_ms: Option<u32>,
    pub last_error: Option<String>,
    pub tunnel_active: bool,
}

#[derive(Default)]
struct ClientStats {
    snapshot: StatsSnapshot,
}

lazy_static::lazy_static! {
    static ref CLIENT_STATS: Mutex<ClientStats> = Mutex::new(ClientStats::default());
}

fn first_server_candidate(value: &str) -> String {
    value
        .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
        .find_map(|candidate| {
            let trimmed = candidate.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .unwrap_or_else(|| value.trim().to_string())
}

pub fn reset(config: &ClientConfig) {
    let primary_server = first_server_candidate(&config.server_url);
    let server_host = url::Url::parse(&primary_server)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_string()))
        .unwrap_or(primary_server);

    let mut stats = CLIENT_STATS.lock().unwrap();
    stats.snapshot = StatsSnapshot {
        state: "starting".to_string(),
        transport: format!("{:?}", config.transport),
        server_host,
        cdn_edge: config.cdn_edge.clone(),
        bytes_up: 0,
        bytes_down: 0,
        active_streams: 0,
        total_streams: 0,
        connected_since: None,
        last_ping_ms: None,
        last_error: None,
        tunnel_active: false,
    };
}

pub fn set_state(state: &str) {
    CLIENT_STATS.lock().unwrap().snapshot.state = state.to_string();
}

pub fn set_error(message: impl Into<String>) {
    let mut stats = CLIENT_STATS.lock().unwrap();
    stats.snapshot.last_error = Some(message.into());
}

pub fn clear_error() {
    CLIENT_STATS.lock().unwrap().snapshot.last_error = None;
}

pub fn mark_transport_connected() {
    let mut stats = CLIENT_STATS.lock().unwrap();
    stats.snapshot.state = "connected".to_string();
    stats.snapshot.tunnel_active = true;
    stats.snapshot.last_error = None;

    if stats.snapshot.connected_since.is_none() {
        stats.snapshot.connected_since = Some(unix_timestamp());
    }
}

pub fn mark_transport_disconnected(error: Option<String>) {
    let mut stats = CLIENT_STATS.lock().unwrap();
    stats.snapshot.state = "disconnected".to_string();
    stats.snapshot.tunnel_active = false;
    stats.snapshot.active_streams = 0;

    if let Some(error) = error {
        stats.snapshot.last_error = Some(error);
    }
}

pub fn note_upstream_bytes(size: usize) {
    CLIENT_STATS.lock().unwrap().snapshot.bytes_up += size as u64;
}

pub fn note_downstream_bytes(size: usize) {
    CLIENT_STATS.lock().unwrap().snapshot.bytes_down += size as u64;
}

pub fn note_stream_open() {
    let mut stats = CLIENT_STATS.lock().unwrap();
    stats.snapshot.total_streams += 1;
    stats.snapshot.active_streams += 1;
}

pub fn note_stream_close() {
    let mut stats = CLIENT_STATS.lock().unwrap();
    stats.snapshot.active_streams = stats.snapshot.active_streams.saturating_sub(1);
}

pub fn note_ping(ms: u32) {
    CLIENT_STATS.lock().unwrap().snapshot.last_ping_ms = Some(ms);
}

pub fn snapshot_json() -> String {
    let snapshot = CLIENT_STATS.lock().unwrap().snapshot.clone();
    serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string())
}

pub struct StreamActivityGuard;

pub fn track_stream() -> StreamActivityGuard {
    note_stream_open();
    StreamActivityGuard
}

impl Drop for StreamActivityGuard {
    fn drop(&mut self) {
        note_stream_close();
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
