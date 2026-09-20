//! HTTP / WebSocket 接口。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use tokio_util::io::ReaderStream;

use crate::config::{Config, Paths};
#[cfg(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda"))]
use crate::engine::Engine;
use crate::engine::{EngineEvent, EngineInfo, EngineService};
use crate::events::{Event, StatusEvent};
use crate::models::{self, ModelStatus};
use crate::session::{ActiveSession, Segment};

static WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web");

/// 一次连接对应的采样率上限，防止异常客户端把内存打爆。
const MAX_FRAME_BYTES: usize = 1 << 20;

pub struct AppState {
    pub paths: Paths,
    /// 实际使用的模型目录（离线包是程序同级 data/models；否则是数据目录下的 models）。
    pub model_dir: PathBuf,
    pub config: Arc<Mutex<Config>>,
    pub session: Arc<Mutex<Option<ActiveSession>>>,
    pub events: broadcast::Sender<Event>,
    pub engine: Arc<EngineService>,
    pub engine_info: Arc<Mutex<EngineInfo>>,
    pub engine_reason: Arc<Mutex<String>>,
    pub model_status: Arc<Mutex<ModelStatus>>,
    /// 最后一次失败原因（引擎装载、模型下载等），界面会一直显示，直到修好。
    pub last_error: Arc<Mutex<Option<String>>>,
    pub recorder: Arc<Mutex<Option<String>>>,
    pub test_api: bool,
}

impl AppState {
    pub fn status(&self) -> StatusEvent {
        let session = self.session.lock().unwrap();
        let config = self.config.lock().unwrap();
        let info = self.engine_info.lock().unwrap().clone();
        let model = self.model_status.lock().unwrap().clone();
        StatusEvent {
            version: env!("CARGO_PKG_VERSION").to_string(),
            session: session.as_ref().map(|s| s.id.clone()),
            sentences: session.as_ref().map(|s| s.sentence_count()).unwrap_or(0),
            segments: session.as_ref().map(|s| s.doc.segments.len()).unwrap_or(0),
            pending_jobs: session.as_ref().map(|s| s.pending_jobs).unwrap_or(0),
            audio_version: session.as_ref().map(|s| s.audio_version).unwrap_or(0),
            provider: info.provider.clone(),
            device: info.device.clone(),
            engine_reason: self.engine_reason.lock().unwrap().clone(),
            model_ready: model.ready,
            model_extras_ready: model.extras_ready,
            model_downloading: model.downloading,
            data_dir: self.paths.data_dir.to_string_lossy().to_string(),
            recordings_dir: self.paths.recordings_dir().to_string_lossy().to_string(),
            model_dir: self.model_dir.to_string_lossy().to_string(),
            model_missing: model.missing.clone(),
            last_error: self.last_error.lock().unwrap().clone(),
            title: session
                .as_ref()
                .map(|s| s.doc.title.clone())
                .unwrap_or_else(|| config.last_title.clone()),
            recording: session.as_ref().map(|s| s.recording).unwrap_or(false),
            recorder_client: self.recorder.lock().unwrap().clone(),
        }
    }

    pub fn broadcast(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/styles.css", get(styles))
        .route("/lib/{*path}", get(lib_asset))
        .route("/api/state", get(get_state))
        .route("/api/session", get(get_session))
        .route("/api/session/start", post(start_session))
        .route("/api/session/finalize", post(finalize_session))
        .route("/api/session/discard", post(discard_session))
        .route("/api/sentences/{id}", axum::routing::patch(patch_sentence))
        .route("/api/sessions/current/audio", get(current_audio))
        .route("/api/models", get(get_models))
        .route("/api/models/download", post(download_models))
        .route("/api/models/rescan", post(rescan_models))
        .route("/api/diagnostics", get(get_diagnostics))
        .route("/api/config", post(set_config))
        .route("/api/recordings", get(list_recordings))
        .route("/api/test/segment", post(test_segment))
        .route("/api/ws", get(ws_handler))
        .with_state(state)
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404").into_response()
}

async fn index() -> Response {
    match WEB.get_file("index.html") {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                // 本地程序：页面必须和当前二进制严格同版，
                // 否则浏览器缓存的旧 app.js 会显示错乱状态（例如「模型没读进来」）。
                (header::CACHE_CONTROL, "no-store, must-revalidate"),
            ],
            file.contents(),
        )
            .into_response(),
        None => not_found(),
    }
}

async fn app_js() -> Response {
    match WEB.get_file("app.js") {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store, must-revalidate"),
            ],
            file.contents(),
        )
            .into_response(),
        None => not_found(),
    }
}

async fn styles() -> Response {
    match WEB.get_file("styles.css") {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store, must-revalidate"),
            ],
            file.contents(),
        )
            .into_response(),
        None => not_found(),
    }
}

async fn lib_asset(Path(path): Path<String>) -> Response {
    let key = format!("lib/{path}");
    match WEB.get_file(&key) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store, must-revalidate"),
            ],
            file.contents(),
        )
            .into_response(),
        None => not_found(),
    }
}

async fn get_state(State(state): State<Arc<AppState>>) -> Json<StatusEvent> {
    Json(state.status())
}

#[derive(Serialize)]
struct SessionView {
    id: String,
    title: String,
    duration_ms: u32,
    audio_version: u64,
    segments: Vec<crate::session::Segment>,
    sentences: Vec<crate::session::Sentence>,
}

/// 当前会话的完整内容（刷新页面 / 重新打开时用来恢复界面）。
async fn get_session(State(state): State<Arc<AppState>>) -> Json<Option<SessionView>> {
    let guard = state.session.lock().unwrap();
    Json(guard.as_ref().map(|session| SessionView {
        id: session.id.clone(),
        title: session.doc.title.clone(),
        duration_ms: session.doc.duration_ms,
        audio_version: session.audio_version,
        segments: session.doc.segments.clone(),
        sentences: session.doc.sentences.clone(),
    }))
}

#[derive(Deserialize)]
struct TitleBody {
    #[serde(default)]
    title: Option<String>,
}

async fn start_session(State(state): State<Arc<AppState>>) -> Response {
    let mut guard = state.session.lock().unwrap();
    if guard.is_some() {
        return Json(state.status()).into_response();
    }
    let info = state.engine_info.lock().unwrap().clone();
    let title = state.config.lock().unwrap().last_title.clone();
    match ActiveSession::create(&state.paths, info, &title) {
        Ok(session) => {
            *guard = Some(session);
            drop(guard);
            state.broadcast(Event::Status(state.status()));
            Json(state.status()).into_response()
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "session_start", error),
    }
}

async fn finalize_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TitleBody>,
) -> Response {
    // 等识别队列跑完（最多 2 分钟）
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let pending = {
            let guard = state.session.lock().unwrap();
            match guard.as_ref() {
                Some(session) => session.pending_jobs,
                None => {
                    return error_response(
                        StatusCode::CONFLICT,
                        "no_session",
                        "当前没有进行中的会话",
                    );
                }
            }
        };
        if pending == 0 || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let title = body
        .title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "语音笔记".to_string());
    let recordings = state.paths.recordings_dir();
    let result = {
        let mut guard = state.session.lock().unwrap();
        match guard.as_mut() {
            Some(session) => session.finalize(&recordings, &title),
            None => {
                return error_response(StatusCode::CONFLICT, "no_session", "当前没有进行中的会话");
            }
        }
    };
    match result {
        Ok(files) => {
            {
                let mut guard = state.session.lock().unwrap();
                *guard = None;
            }
            {
                let mut config = state.config.lock().unwrap();
                config.last_title = title;
                let _ = config.save(&state.paths.data_dir);
            }
            state.broadcast(Event::Status(state.status()));
            Json(json!({
                "files": files.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>()
            }))
            .into_response()
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "finalize", error),
    }
}

async fn discard_session(State(state): State<Arc<AppState>>) -> Response {
    let result = {
        let mut guard = state.session.lock().unwrap();
        match guard.as_mut() {
            Some(session) => session.discard(),
            None => Ok(()),
        }
    };
    match result {
        Ok(()) => {
            *state.session.lock().unwrap() = None;
            state.broadcast(Event::Status(state.status()));
            Json(json!({"ok": true})).into_response()
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "discard", error),
    }
}

#[derive(Deserialize)]
struct SentenceBody {
    text: String,
}

async fn patch_sentence(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SentenceBody>,
) -> Response {
    let result = {
        let mut guard = state.session.lock().unwrap();
        match guard.as_mut() {
            Some(session) => session.update_sentence(&id, &body.text),
            None => return error_response(StatusCode::CONFLICT, "no_session", "当前没有会话"),
        }
    };
    match result {
        Ok(sentence) => {
            state.broadcast(Event::Status(state.status()));
            Json(sentence).into_response()
        }
        Err(error) => error_response(StatusCode::NOT_FOUND, "sentence", error),
    }
}

/// 合并音频：支持 Range，录制中也能边录边播。
async fn current_audio(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let path: Option<PathBuf> = {
        let guard = state.session.lock().unwrap();
        guard.as_ref().map(|session| session.dir.join("audio.wav"))
    };
    let Some(path) = path else {
        return error_response(StatusCode::CONFLICT, "no_session", "当前没有会话");
    };
    serve_wav(&path, &headers).await
}

async fn serve_wav(path: &std::path::Path, headers: &HeaderMap) -> Response {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(_) => return not_found(),
    };
    let total = metadata.len();
    if total == 0 {
        return error_response(StatusCode::CONFLICT, "empty_audio", "还没有录音");
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, total));
    match range {
        Some((start, end)) => {
            let length = end - start + 1;
            let file = match tokio::fs::File::open(path).await {
                Ok(file) => file,
                Err(_) => return not_found(),
            };
            let mut reader = tokio::io::BufReader::new(file);
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            if reader.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "seek", "读取音频失败");
            }
            let stream = ReaderStream::new(reader.take(length));
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, "audio/wav")
                .header(header::ACCEPT_RANGES, "bytes")
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{total}"),
                )
                .header(header::CONTENT_LENGTH, length)
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| not_found())
        }
        None => {
            let file = match tokio::fs::File::open(path).await {
                Ok(file) => file,
                Err(_) => return not_found(),
            };
            let stream = ReaderStream::new(file);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "audio/wav")
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, total)
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| not_found())
        }
    }
}

/// `bytes=start-end`（end 可省略）。
pub fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let first = spec.split(',').next()?;
    let (start, end) = first.split_once('-')?;
    let (start, end) = if start.is_empty() {
        // 后缀范围：bytes=-N
        let suffix: u64 = end.trim().parse().ok()?;
        (total.saturating_sub(suffix), total.saturating_sub(1))
    } else {
        let start: u64 = start.trim().parse().ok()?;
        let end = if end.trim().is_empty() {
            total.saturating_sub(1)
        } else {
            end.trim().parse::<u64>().ok()?.min(total.saturating_sub(1))
        };
        (start, end)
    };
    (start <= end && start < total).then_some((start, end))
}

async fn get_models(State(state): State<Arc<AppState>>) -> Json<ModelStatus> {
    let status = models::status(&state.model_dir);
    *state.model_status.lock().unwrap() = status.clone();
    Json(status)
}

/// 完整诊断：程序在哪个目录找模型、每个文件在不在、校验过不过。
/// 排查「模型文件明明在、网页却说不认识」时先看这个。
async fn get_diagnostics(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let diagnostics = models::diagnose(&state.model_dir);
    let info = state.engine_info.lock().unwrap().clone();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "exe_path": std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        "data_dir": state.paths.data_dir.to_string_lossy().to_string(),
        "data_dir_is_portable": state.paths.portable,
        "model_dir": state.model_dir.to_string_lossy().to_string(),
        "recordings_dir": state.paths.recordings_dir().to_string_lossy().to_string(),
        "bundled_models_dir": state.paths.bundled_models_dir().to_string_lossy().to_string(),
        "log_file": state.paths.data_dir.join("mind_flow.log").to_string_lossy().to_string(),
        "engine": info,
        "engine_reason": state.engine_reason.lock().unwrap().clone(),
        "last_error": state.last_error.lock().unwrap().clone(),
        "models": diagnostics,
    }))
}

/// 重新扫描模型（把模型拷进目录后不必重启程序）。
async fn rescan_models(State(state): State<Arc<AppState>>) -> Response {
    let status = models::status(&state.model_dir);
    let ready = status.ready;
    *state.model_status.lock().unwrap() = status.clone();
    if ready {
        reload_engine(&state).await;
        state.last_error.lock().unwrap().take();
    }
    state.broadcast(Event::Status(state.status()));
    Json(json!({
        "ready": ready,
        "extras_ready": status.extras_ready,
        "missing": status.missing,
        "model_dir": state.model_dir.to_string_lossy().to_string(),
    }))
    .into_response()
}

async fn download_models(State(state): State<Arc<AppState>>) -> Response {
    if spawn_model_download(&state) {
        Json(json!({"started": true})).into_response()
    } else {
        Json(json!({"started": false, "reason": "已在下载中"})).into_response()
    }
}

/// 触发模型下载（已在下载中时返回 false）。
pub fn spawn_model_download(state: &Arc<AppState>) -> bool {
    if state.model_status.lock().unwrap().downloading {
        return false;
    }
    state.model_status.lock().unwrap().downloading = true;
    let state_clone = state.clone();
    tokio::spawn(async move {
        let config = state_clone.config.lock().unwrap().clone();
        let models_dir = state_clone.model_dir.clone();
        let downloads_dir = state_clone.paths.downloads_dir();
        let events = state_clone.events.clone();
        let result = models::download_all(&models_dir, &downloads_dir, &config, move |progress| {
            let _ = events.send(Event::ModelProgress {
                file: progress.file,
                downloaded: progress.downloaded,
                total: progress.total,
                finished: progress.finished,
            });
        })
        .await;
        state_clone.model_status.lock().unwrap().downloading = false;
        match result {
            Ok(()) => {
                let status = models::status(&models_dir);
                *state_clone.model_status.lock().unwrap() = status;
                state_clone.last_error.lock().unwrap().take();
                reload_engine(&state_clone).await;
                state_clone.broadcast(Event::Status(state_clone.status()));
            }
            Err(error) => {
                *state_clone.last_error.lock().unwrap() = Some(format!("模型下载失败：{error}"));
                state_clone.broadcast(Event::Error {
                    code: "model_download".into(),
                    message: error.to_string(),
                });
            }
        }
    });
    true
}

/// 模型就绪后装载 CPU 引擎（GPU 探测由 gpu 模块另行处理）。
pub async fn reload_engine(state: &Arc<AppState>) {
    let paths = models::ModelPaths::resolve(&state.model_dir);
    if !paths.is_ready() {
        return;
    }
    let threads = std::thread::available_parallelism()
        .map(|value| value.get().min(4) as i32)
        .unwrap_or(2);
    let label = paths
        .asr_model
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "paraformer-zh".to_string());
    #[cfg(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda"))]
    {
        let provider = "cpu";
        match crate::engine::sherpa::SherpaEngine::load(
            &paths,
            provider,
            threads,
            "CPU".to_string(),
            label,
        ) {
            Ok(engine) => {
                *state.engine_info.lock().unwrap() = engine.info();
                *state.engine_reason.lock().unwrap() = "内置 CPU 引擎".to_string();
                state.last_error.lock().unwrap().take();
                state.engine.swap(Box::new(engine));
            }
            Err(error) => {
                let message = format!(
                    "装载识别引擎失败：{error}（模型目录：{}）",
                    state.model_dir.display()
                );
                *state.last_error.lock().unwrap() = Some(message.clone());
                state.broadcast(Event::Error {
                    code: "engine_load".into(),
                    message,
                });
            }
        }
    }
    #[cfg(not(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda")))]
    {
        let _ = (paths, threads, label);
    }
}

#[derive(Serialize)]
struct RecordingEntry {
    name: String,
    base: String,
    size: u64,
    modified: String,
}

async fn list_recordings(State(state): State<Arc<AppState>>) -> Json<Vec<RecordingEntry>> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(state.paths.recordings_dir()) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let base = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let metadata = entry.metadata().ok();
            entries.push(RecordingEntry {
                name: path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                base,
                size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: metadata
                    .and_then(|m| m.modified().ok())
                    .map(|time| {
                        let datetime: chrono::DateTime<chrono::Local> = time.into();
                        datetime.to_rfc3339()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    entries.sort_by(|a, b| b.modified.cmp(&a.modified));
    Json(entries)
}

/// 测试/冒烟用：直接把一段 PCM 当成录好的段落送进识别（仅 `--engine stub` 或显式打开时可用）。
async fn test_segment(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TestQuery>,
    body: axum::body::Bytes,
) -> Response {
    if !state.test_api {
        return error_response(StatusCode::FORBIDDEN, "test_api_disabled", "测试接口未启用");
    }
    if body.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "empty", "音频为空");
    }
    let rate = query.rate.unwrap_or(crate::audio::TARGET_RATE);
    // 直接喂 WAV 文件也行（冒烟脚本与手动验证用）
    let (body, rate) = if body.starts_with(b"RIFF") {
        match crate::audio::parse_wav_mono16(&body) {
            Ok((samples, rate)) => (
                axum::body::Bytes::from(crate::audio::i16_to_bytes_le(&samples)),
                rate,
            ),
            Err(error) => {
                return error_response(StatusCode::BAD_REQUEST, "bad_wav", error);
            }
        }
    } else {
        (body, rate)
    };
    let mut guard = state.session.lock().unwrap();
    if guard.is_none() {
        let info = state.engine_info.lock().unwrap().clone();
        let title = state.config.lock().unwrap().last_title.clone();
        match ActiveSession::create(&state.paths, info, &title) {
            Ok(session) => *guard = Some(session),
            Err(error) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "create", error);
            }
        }
    }
    let session = guard.as_mut().unwrap();
    let result = (|| -> anyhow::Result<Option<Segment>> {
        session.begin_segment(rate)?;
        session.append_pcm(&body)?;
        let finished = session.end_segment(rate)?;
        Ok(finished.map(|f| f.segment))
    })();
    match result {
        Ok(maybe_segment) => {
            let segment = match maybe_segment {
                Some(segment) => segment,
                None => {
                    return Json(json!({"segment": null, "reason": "too_short"})).into_response();
                }
            };
            // 重新读一遍 16k PCM 交给识别队列
            let pcm_path = session.dir.join("seg").join(format!("{}.wav", segment.id));
            let pcm = crate::audio::read_wav_mono16(&pcm_path).map(|(samples, _)| samples);
            session.pending_jobs += 1;
            let id = segment.id;
            drop(guard);
            match pcm {
                Ok(pcm) => state.engine.submit(id, pcm),
                Err(error) => {
                    state.broadcast(Event::Error {
                        code: "segment_read".into(),
                        message: error.to_string(),
                    });
                }
            }
            state.broadcast(Event::SegmentClosed {
                segment: segment.clone(),
            });
            Json(json!({"segment": segment})).into_response()
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "segment", error),
    }
}

#[derive(Deserialize)]
struct TestQuery {
    rate: Option<u32>,
}

#[derive(Deserialize)]
struct ConfigBody {
    inference: Option<String>,
}

/// 界面里改设置（目前只有推理设备偏好，改动下次启动生效）。
async fn set_config(State(state): State<Arc<AppState>>, Json(body): Json<ConfigBody>) -> Response {
    {
        let mut config = state.config.lock().unwrap();
        if let Some(inference) = body.inference.as_deref() {
            match crate::config::Inference::parse(inference) {
                Some(parsed) => config.inference = parsed,
                None => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "bad_inference",
                        format!("不认识的推理设备：{inference}"),
                    );
                }
            }
        }
        if let Err(error) = config.save(&state.paths.data_dir) {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "save_config", error);
        }
    }
    Json(json!({"ok": true, "note": "推理设备改动在下次启动时生效"})).into_response()
}

#[derive(Deserialize)]
struct WsQuery {
    rate: Option<u32>,
}

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| {
        ws_loop(
            socket,
            state,
            query.rate.unwrap_or(crate::audio::TARGET_RATE),
        )
    })
}

async fn ws_loop(socket: WebSocket, state: Arc<AppState>, rate: u32) {
    let (mut sink, mut stream) = socket.split();
    let client_id = format!("c-{:04x}", rand::random::<u16>());
    {
        let mut recorder = state.recorder.lock().unwrap();
        if recorder.is_none() {
            *recorder = Some(client_id.clone());
        }
    }
    let mut events = state.events.subscribe();
    // 先告诉这个连接它是谁、拿没拿到录音权
    let is_recorder = state.recorder.lock().unwrap().as_deref() == Some(client_id.as_str());
    let hello = json!({
        "type": "hello",
        "client_id": client_id,
        "recorder": is_recorder,
    })
    .to_string();
    let initial = serde_json::to_string(&Event::Status(state.status())).unwrap_or_default();
    if sink.send(Message::Text(hello.into())).await.is_err() {
        return;
    }
    if sink.send(Message::Text(initial.into())).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        let is_recorder = state.recorder.lock().unwrap().as_deref() == Some(client_id.as_str());
                        if let Err(error) = handle_control(&state, &text, rate, is_recorder).await {
                            let payload = serde_json::to_string(&Event::Error {
                                code: "control".into(),
                                message: error.to_string(),
                            }).unwrap_or_default();
                            if sink.send(Message::Text(payload.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Message::Binary(bytes) => {
                        let is_recorder = state.recorder.lock().unwrap().as_deref() == Some(client_id.as_str());
                        if is_recorder && bytes.len() <= MAX_FRAME_BYTES {
                            if let Err(error) = append_pcm(&state, &bytes) {
                                let payload = serde_json::to_string(&Event::Error {
                                    code: "pcm".into(),
                                    message: error.to_string(),
                                }).unwrap_or_default();
                                if sink.send(Message::Text(payload.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        let payload = serde_json::to_string(&event).unwrap_or_default();
                        if sink.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    {
        let mut recorder = state.recorder.lock().unwrap();
        if recorder.as_deref() == Some(client_id.as_str()) {
            *recorder = None;
        }
    }
    // 客户端掉线时把没结束的段落收尾，已录到的音频不丢
    let _ = finish_segment(&state, rate, true);
}

async fn handle_control(
    state: &Arc<AppState>,
    text: &str,
    rate: u32,
    is_recorder: bool,
) -> anyhow::Result<()> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    match value.get("type").and_then(|v| v.as_str()) {
        Some("start") => {
            if !is_recorder {
                anyhow::bail!("另一个标签页正在录音");
            }
            ensure_session(state)?;
            {
                let mut guard = state.session.lock().unwrap();
                if let Some(session) = guard.as_mut() {
                    if !session.recording {
                        session.begin_segment(rate)?;
                    }
                }
            }
            state.broadcast(Event::Status(state.status()));
        }
        Some("stop") => {
            if !is_recorder {
                return Ok(());
            }
            finish_segment(state, rate, false)?;
        }
        Some("claim_recorder") => {
            let mut recorder = state.recorder.lock().unwrap();
            if recorder.is_none() {
                *recorder = Some("claimed".to_string());
            }
            state.broadcast(Event::Status(state.status()));
        }
        _ => {}
    }
    Ok(())
}

fn ensure_session(state: &Arc<AppState>) -> anyhow::Result<()> {
    let mut guard = state.session.lock().unwrap();
    if guard.is_none() {
        let info = state.engine_info.lock().unwrap().clone();
        let title = state.config.lock().unwrap().last_title.clone();
        *guard = Some(ActiveSession::create(&state.paths, info, &title)?);
    }
    Ok(())
}

fn append_pcm(state: &Arc<AppState>, bytes: &[u8]) -> anyhow::Result<()> {
    let mut guard = state.session.lock().unwrap();
    let Some(session) = guard.as_mut() else {
        anyhow::bail!("当前没有会话");
    };
    session.append_pcm(bytes)
}

/// 收尾当前段落并把 16k PCM 送进识别队列。
fn finish_segment(state: &Arc<AppState>, rate: u32, silent: bool) -> anyhow::Result<()> {
    let (segment, pcm) = {
        let mut guard = state.session.lock().unwrap();
        let Some(session) = guard.as_mut() else {
            return Ok(());
        };
        let Some(finished) = session.end_segment(rate)? else {
            if !silent {
                state.broadcast(Event::Status(state.status()));
            }
            return Ok(());
        };
        session.pending_jobs += 1;
        (finished.segment, finished.pcm16k)
    };
    state.engine.submit(segment.id, pcm);
    state.broadcast(Event::SegmentClosed {
        segment: segment.clone(),
    });
    state.broadcast(Event::Status(state.status()));
    Ok(())
}

/// 引擎事件 → 会话状态 + 广播。
pub async fn engine_event_loop(
    state: Arc<AppState>,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<EngineEvent>,
) {
    while let Some(event) = receiver.recv().await {
        match event {
            EngineEvent::JobDone {
                segment_id,
                sentences,
                info,
                ..
            } => {
                let added = {
                    let mut guard = state.session.lock().unwrap();
                    match guard.as_mut() {
                        Some(session) => {
                            session.pending_jobs = session.pending_jobs.saturating_sub(1);
                            let added = session.add_sentences(segment_id, sentences, info).ok();
                            let version = session.audio_version;
                            added.map(|sentences| (sentences, version))
                        }
                        None => None,
                    }
                };
                if let Some((sentences, audio_version)) = added {
                    state.broadcast(Event::SentencesAdded {
                        sentences,
                        audio_version,
                    });
                }
                state.broadcast(Event::Status(state.status()));
            }
            EngineEvent::JobFailed { segment_id, error } => {
                {
                    let mut guard = state.session.lock().unwrap();
                    if let Some(session) = guard.as_mut() {
                        session.pending_jobs = session.pending_jobs.saturating_sub(1);
                        let _ = session.mark_segment_failed(segment_id);
                    }
                }
                state.broadcast(Event::Error {
                    code: "recognize".into(),
                    message: format!("第 {segment_id} 段识别失败：{error}"),
                });
                state.broadcast(Event::Status(state.status()));
            }
            EngineEvent::EngineChanged { info, reason } => {
                *state.engine_info.lock().unwrap() = info.clone();
                *state.engine_reason.lock().unwrap() = reason.clone();
                state.broadcast(Event::EngineChanged {
                    provider: info.provider,
                    device: info.device,
                    reason,
                });
                state.broadcast(Event::Status(state.status()));
            }
        }
    }
}

fn error_response(status: StatusCode, code: &str, message: impl std::fmt::Display) -> Response {
    (
        status,
        Json(json!({"error": code, "message": message.to_string()})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range解析() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 1000), Some((100, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-99999", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bogus", 1000), None);
    }
}
