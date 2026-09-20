//! 集成测试：启动真实服务（stub 引擎），走一遍 HTTP + WS 的完整流程。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mind_flow::api::{self, AppState};
use mind_flow::audio;
use mind_flow::config::{Config, Paths};
use mind_flow::engine::{EngineInfo, EngineService, StubEngine};
use mind_flow::models;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

struct TestServer {
    base: String,
    data_dir: std::path::PathBuf,
    state: Arc<AppState>,
}

async fn start(name: &str) -> TestServer {
    start_with(name, true).await
}

async fn start_with(name: &str, test_api: bool) -> TestServer {
    let data_dir = std::env::temp_dir().join(format!(
        "mind_flow_http_{name}_{}_{:04x}",
        std::process::id(),
        rand::random::<u16>()
    ));
    let _ = std::fs::remove_dir_all(&data_dir);
    let paths = Paths {
        data_dir: data_dir.clone(),
        portable: true,
    };
    paths.ensure_dirs().unwrap();

    let (engine_tx, engine_rx) = mpsc::unbounded_channel();
    let engine = EngineService::spawn(engine_tx, Some(Box::new(StubEngine::new())), Vec::new());
    let (events, _) = tokio::sync::broadcast::channel(128);
    let state = Arc::new(AppState {
        paths: paths.clone(),
        config: Arc::new(Mutex::new(Config::default())),
        session: Arc::new(Mutex::new(None)),
        events,
        engine,
        engine_info: Arc::new(Mutex::new(EngineInfo::default())),
        engine_reason: Arc::new(Mutex::new("测试桩".into())),
        model_status: Arc::new(Mutex::new(models::status(&paths.models_dir()))),
        recorder: Arc::new(Mutex::new(None)),
        test_api,
    });
    tokio::spawn(api::engine_event_loop(state.clone(), engine_rx));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = api::router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    TestServer {
        base: format!("http://{address}"),
        data_dir,
        state,
    }
}

impl TestServer {
    async fn get_json(&self, path: &str) -> serde_json::Value {
        let response = reqwest::get(format!("{}{path}", self.base)).await.unwrap();
        response.json().await.unwrap()
    }
}

/// 生成指定采样率的方波 PCM（模拟设备原始采样率）。
fn 说话声(ms: u32, rate: u32) -> Vec<u8> {
    let samples: Vec<i16> = (0..(rate as u64 * ms as u64 / 1000) as usize)
        .map(|i| if (i / 40) % 2 == 0 { 8000 } else { -8000 })
        .collect();
    audio::i16_to_bytes_le(&samples)
}

/// 默认 16kHz 的说话声。
fn 说话声16k(ms: u32) -> Vec<u8> {
    说话声(ms, 16_000)
}

/// 等到识别完成（pending_jobs 归零且有句子）。
async fn 等识别(server: &TestServer, 期望句数: usize) -> serde_json::Value {
    let mut last = serde_json::Value::Null;
    for _ in 0..100 {
        let state = server.get_json("/api/state").await;
        last = state.clone();
        if state["pending_jobs"].as_u64() == Some(0)
            && state["sentences"].as_u64() == Some(期望句数 as u64)
        {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("等待识别超时（期望 {期望句数} 句），最后状态：{last}");
}

#[tokio::test]
async fn 服务能启动并报告状态() {
    let server = start("state").await;
    let state = server.get_json("/api/state").await;
    assert!(state["model_ready"].is_boolean());
    assert_eq!(state["pending_jobs"], 0);
    assert_eq!(state["session"], serde_json::Value::Null);
    let index = reqwest::get(format!("{}/", server.base)).await.unwrap();
    assert_eq!(index.status(), 200);
    let html = index.text().await.unwrap();
    assert!(html.contains("mind_flow"), "首页应内嵌前端");
    assert!(html.contains("btn-finish"), "首页应包含保存按钮");
    let _ = std::fs::remove_dir_all(&server.data_dir);
}

#[tokio::test]
async fn 测试接口能录音识别并导出四个文件() {
    let server = start("flow").await;
    // 第一段
    let response = reqwest::Client::new()
        .post(format!("{}/api/test/segment?rate=16000", server.base))
        .body(说话声16k(1200))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let state = 等识别(&server, 1).await;
    assert_eq!(state["segments"], 1);
    assert!(state["audio_version"].as_u64().unwrap() >= 1);

    // 第二段：时间戳应接着第一段
    let second = reqwest::Client::new()
        .post(format!("{}/api/test/segment?rate=48000", server.base))
        .body(说话声(600, 48_000))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200, "第二段应被接受（48kHz 设备采样率）");
    let state = 等识别(&server, 2).await;
    assert_eq!(state["segments"], 2);

    let session = server.state.session.lock().unwrap();
    let doc = &session.as_ref().unwrap().doc;
    let second_start = doc.sentences.last().unwrap().start_ms;
    assert!(
        second_start >= 1400,
        "第二段应在 400ms 静音之后（实际 {second_start}ms）"
    );
    let first_end = doc.sentences[0].end_ms;
    assert!(second_start >= first_end, "句子时间戳应单调");
    drop(session);

    // 编辑纠错
    let sentence_id = {
        let session = server.state.session.lock().unwrap();
        session.as_ref().unwrap().doc.sentences[0].id.clone()
    };
    let patched = reqwest::Client::new()
        .patch(format!("{}/api/sentences/{sentence_id}", server.base))
        .json(&serde_json::json!({"text": "改好的第一句。"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patched.status(), 200);

    // 保存
    let response = reqwest::Client::new()
        .post(format!("{}/api/session/finalize", server.base))
        .json(&serde_json::json!({"title": "集成测试"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let payload: serde_json::Value = response.json().await.unwrap();
    let files: Vec<String> = payload["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    assert_eq!(files.len(), 4);
    for file in &files {
        assert!(std::path::Path::new(file).exists(), "缺少产物 {file}");
    }
    // .txt 里是改过的文字，.json 里是完整结构
    let txt = std::fs::read_to_string(&files[1]).unwrap();
    assert!(txt.contains("改好的第一句。"), "实际：{txt}");
    assert_eq!(txt.lines().count(), 2, "两句一行一句：{txt}");
    assert!(txt.contains("第1句"), "第二段的文本也应在：{txt}");
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&files[2]).unwrap()).unwrap();
    assert_eq!(json["title"], "集成测试");
    assert_eq!(json["sample_rate"], 16000);
    assert_eq!(json["segments"].as_array().unwrap().len(), 2);
    let srt = std::fs::read_to_string(&files[3]).unwrap();
    assert!(srt.contains("-->"), "srt 应有时间轴：{srt}");
    let (samples, rate) = audio::read_wav_mono16(std::path::Path::new(&files[0])).unwrap();
    assert_eq!(rate, 16000);
    assert!(samples.len() > 16_000, "合并音频应包含两段与静音");
    // 保存后没有进行中的会话
    let state = server.get_json("/api/state").await;
    assert_eq!(state["session"], serde_json::Value::Null);
    let _ = std::fs::remove_dir_all(&server.data_dir);
}

#[tokio::test]
async fn 音频支持range请求() {
    let server = start("range").await;
    reqwest::Client::new()
        .post(format!("{}/api/test/segment?rate=16000", server.base))
        .body(说话声16k(1000))
        .send()
        .await
        .unwrap();
    等识别(&server, 1).await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/sessions/current/audio?v=1", server.base))
        .header("range", "bytes=0-43")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        response
            .headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("bytes 0-43/"),
        true
    );
    let body = response.bytes().await.unwrap();
    assert_eq!(&body[0..4], b"RIFF");
    let _ = std::fs::remove_dir_all(&server.data_dir);
}

#[tokio::test]
async fn 丢弃会话会清空状态并删目录() {
    let server = start("discard").await;
    reqwest::Client::new()
        .post(format!("{}/api/test/segment?rate=16000", server.base))
        .body(说话声16k(800))
        .send()
        .await
        .unwrap();
    等识别(&server, 1).await;
    let dir = {
        let session = server.state.session.lock().unwrap();
        session.as_ref().unwrap().dir.clone()
    };
    reqwest::Client::new()
        .post(format!("{}/api/session/discard", server.base))
        .send()
        .await
        .unwrap();
    let state = server.get_json("/api/state").await;
    assert_eq!(state["session"], serde_json::Value::Null);
    assert_eq!(state["sentences"], 0);
    assert!(!dir.exists(), "会话目录应被删除");
    let _ = std::fs::remove_dir_all(&server.data_dir);
}

#[tokio::test]
async fn 刷新页面能取回完整会话() {
    let server = start("session_view").await;
    // 没有会话时返回 null
    assert_eq!(
        server.get_json("/api/session").await,
        serde_json::Value::Null
    );
    reqwest::Client::new()
        .post(format!("{}/api/test/segment?rate=16000", server.base))
        .body(说话声16k(900))
        .send()
        .await
        .unwrap();
    等识别(&server, 1).await;
    let view = server.get_json("/api/session").await;
    assert!(view["id"].is_string());
    assert_eq!(view["sentences"].as_array().unwrap().len(), 1);
    assert_eq!(view["segments"].as_array().unwrap().len(), 1);
    assert!(view["audio_version"].as_u64().unwrap() >= 1);
    assert!(view["duration_ms"].as_u64().unwrap() >= 900);
    let _ = std::fs::remove_dir_all(&server.data_dir);
}

#[tokio::test]
async fn websocket能录音并收到句子事件() {
    let server = start("ws").await;
    let url = server.base.replace("http://", "ws://") + "/api/ws?rate=16000";
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // 首帧应是 hello，随后是 status
    let hello = 下一帧(&mut socket).await;
    assert_eq!(hello["type"], "hello");
    let status = 下一帧(&mut socket).await;
    assert_eq!(status["type"], "status");

    socket
        .send(Message::Text(
            serde_json::json!({"type": "start"}).to_string().into(),
        ))
        .await
        .unwrap();
    // 分两帧送 PCM，模拟实时流
    let pcm = 说话声16k(900);
    let (head, tail) = pcm.split_at(pcm.len() / 2);
    socket
        .send(Message::Binary(head.to_vec().into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    socket
        .send(Message::Binary(tail.to_vec().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "stop"}).to_string().into(),
        ))
        .await
        .unwrap();

    // 等待 sentences_added
    let mut got_sentences = false;
    for _ in 0..40 {
        let event = 下一帧(&mut socket).await;
        if event["type"] == "sentences_added" {
            assert!(event["sentences"].as_array().unwrap().len() >= 1);
            assert!(event["audio_version"].as_u64().unwrap() >= 1);
            got_sentences = true;
            break;
        }
    }
    assert!(got_sentences, "应收到 sentences_added 事件");
    let _ = std::fs::remove_dir_all(&server.data_dir);
}

async fn 下一帧(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> serde_json::Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("等待 WS 消息超时")
            .expect("WS 连接关闭")
            .expect("WS 错误");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

#[tokio::test]
async fn 测试接口在未启用时拒绝访问() {
    let server = start_with("disabled", false).await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/test/segment?rate=16000", server.base))
        .body(说话声16k(500))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let _ = std::fs::remove_dir_all(&server.data_dir);
}
