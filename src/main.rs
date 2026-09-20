//! mind_flow 主程序：启动本地服务 + 内嵌界面，识别与落盘全在本机。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;
use mind_flow::api::{self, AppState};
use mind_flow::config::{Config, Inference, Paths};
use mind_flow::engine::{Engine, EngineInfo, EngineService, StubEngine};
use mind_flow::events::Event;
use mind_flow::models::{self, ModelPaths};
use mind_flow::session::ActiveSession;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "mind_flow",
    version,
    about = "本地语音笔记：按住空格说话，松开出字，每句一个可回跳的时间戳"
)]
struct Cli {
    /// 数据目录（默认程序同级 data/，不可写时回退系统目录）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 模型目录（默认 <data>/models）
    #[arg(long)]
    model_dir: Option<PathBuf>,
    /// 监听端口（默认 8730，被占用则递增）
    #[arg(long)]
    port: Option<u16>,
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// 不自动打开浏览器
    #[arg(long)]
    no_open: bool,
    /// 推理设备：auto 优先 CUDA，不可用回落 CPU
    #[arg(long)]
    inference: Option<String>,
    /// 识别引擎：real（默认）或 stub（确定性测试桩，同时开启测试接口）
    #[arg(long, default_value = "real")]
    engine: String,
    /// 打开测试接口（`POST /api/test/segment`，供脚本冒烟用）
    #[arg(long, hide = true)]
    test_api: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let paths = Paths::resolve(cli.data_dir.as_deref());
    paths.ensure_dirs()?;
    let mut config = Config::load(&paths.data_dir);
    if let Some(port) = cli.port {
        config.port = port;
    }
    if cli.no_open {
        config.open_browser = false;
    }
    if let Some(inference) = cli.inference.as_deref() {
        if let Some(parsed) = Inference::parse(inference) {
            config.inference = parsed;
        }
    }
    config.save(&paths.data_dir)?;
    if !paths.portable {
        tracing::warn!("程序目录不可写，数据放到 {}", paths.data_dir.display());
    }

    let models_dir = cli.model_dir.clone().unwrap_or_else(|| paths.models_dir());
    let stub = cli.engine == "stub";

    // 引擎：stub 直接可用；real 在模型齐全时同步装载 CPU 引擎
    let (initial_engine, queued_jobs, model_paths) = prepare_engine(&paths, &models_dir, stub)?;

    let (engine_tx, engine_rx) = mpsc::unbounded_channel();
    let engine = EngineService::spawn(engine_tx, initial_engine, queued_jobs);
    let (events, _) = tokio::sync::broadcast::channel(256);

    let state = Arc::new(AppState {
        paths: paths.clone(),
        config: Arc::new(Mutex::new(config.clone())),
        session: Arc::new(Mutex::new(None)),
        events,
        engine,
        engine_info: Arc::new(Mutex::new(EngineInfo::default())),
        engine_reason: Arc::new(Mutex::new("正在装载模型".to_string())),
        model_status: Arc::new(Mutex::new(models::status(&models_dir))),
        recorder: Arc::new(Mutex::new(None)),
        test_api: stub || cli.test_api,
    });

    tokio::spawn(api::engine_event_loop(state.clone(), engine_rx));

    // 恢复未完成的会话
    if let Some(dir) = ActiveSession::find_unfinished(&paths) {
        let info = state.engine_info.lock().unwrap().clone();
        match ActiveSession::resume(&paths, &dir, info) {
            Ok((session, recovery)) => {
                let recovered = session.doc.sentences.len();
                tracing::info!(
                    "恢复会话 {}（{} 句，{} 段待重新识别）",
                    session.id,
                    recovered,
                    recovery.len()
                );
                *state.session.lock().unwrap() = Some(session);
                requeue(&state, recovery);
            }
            Err(error) => tracing::warn!("恢复会话失败：{error}"),
        }
    }

    // 模型缺失时自动开始下载
    if !stub && !state.model_status.lock().unwrap().ready {
        tracing::info!("模型不完整，开始自动下载");
        api::spawn_model_download(&state);
    }

    // GPU：后台探测，成功就切到 CUDA 引擎
    let gpu_state = state.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    if !stub && state.config.lock().unwrap().inference != Inference::Cpu {
        tokio::spawn(gpu_task(gpu_state, shutdown_rx, model_paths));
    }

    let listener = bind_listener(&cli.host, config.port).await?;
    let address = listener.local_addr()?;
    let url = format!("http://{}", address);
    tracing::info!("mind_flow 已启动：{url}");
    tracing::info!("数据目录：{}", paths.data_dir.display());
    if config.open_browser {
        open_browser(&url);
    }

    let app = api::router(state.clone());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP 服务异常退出")?;
    let _ = shutdown_tx.send(true); // 让 CUDA 引擎子进程退出
    Ok(())
}

/// 准备识别引擎与需要重新识别的段落。
fn prepare_engine(
    paths: &Paths,
    models_dir: &std::path::Path,
    stub: bool,
) -> Result<(Option<Box<dyn Engine>>, Vec<(u32, Vec<i16>)>, ModelPaths)> {
    let model_paths = ModelPaths::resolve(models_dir);
    if stub {
        return Ok((Some(Box::new(StubEngine::new())), Vec::new(), model_paths));
    }
    let mut queued = Vec::new();
    if let Some(dir) = ActiveSession::find_unfinished(paths) {
        // 先把恢复出来的段落读进内存，交给引擎线程排队
        for entry in std::fs::read_dir(dir.join("seg"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u32>().ok())
            else {
                continue;
            };
            if let Ok((samples, _)) = mind_flow::audio::read_wav_mono16(&path) {
                queued.push((id, samples));
            }
        }
    }
    #[cfg(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda"))]
    {
        if model_paths.is_ready() {
            let threads = std::thread::available_parallelism()
                .map(|value| value.get().min(4) as i32)
                .unwrap_or(2);
            let label = model_paths
                .asr_model
                .parent()
                .and_then(|dir| dir.file_name())
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "paraformer-zh".to_string());
            match mind_flow::engine::sherpa::SherpaEngine::load(
                &model_paths,
                "cpu",
                threads,
                "CPU".to_string(),
                label,
            ) {
                Ok(engine) => {
                    tracing::info!("CPU 引擎就绪（{} 线程）", threads);
                    return Ok((Some(Box::new(engine)), queued, model_paths));
                }
                Err(error) => tracing::warn!("装载 CPU 引擎失败：{error}"),
            }
        } else {
            tracing::info!("模型尚未就绪，识别任务会排队等待");
        }
    }
    #[cfg(not(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda")))]
    {
        if !model_paths.is_ready() {
            tracing::info!("模型尚未就绪，识别任务会排队等待");
        }
        tracing::warn!("本次构建未启用 sherpa 特性，只能跑测试桩");
    }
    Ok((None, queued, model_paths))
}

fn requeue(state: &Arc<AppState>, jobs: Vec<(u32, PathBuf)>) {
    for (segment_id, path) in jobs {
        match mind_flow::audio::read_wav_mono16(&path) {
            Ok((samples, _)) => {
                if let Some(session) = state.session.lock().unwrap().as_mut() {
                    session.pending_jobs += 1;
                }
                state.engine.submit(segment_id, samples);
            }
            Err(error) => tracing::warn!("读取待识别音频失败：{error}"),
        }
    }
}

/// 后台探测 CUDA 引擎；成功则切换到 GPU 并保持子进程存活。
async fn gpu_task(
    state: Arc<AppState>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    _model_paths: ModelPaths,
) {
    let engine_path = mind_flow::engine::gpu::cuda_engine_path(&state.paths.runtime_cuda_dir());
    let models_dir = state.paths.models_dir();
    if !engine_path.exists() {
        *state.engine_reason.lock().unwrap() = "未安装 CUDA 引擎附件，使用 CPU".to_string();
        state.broadcast(Event::Status(state.status()));
        return;
    }
    match mind_flow::engine::gpu::probe(
        &engine_path,
        &models_dir,
        std::time::Duration::from_secs(8),
    )
    .await
    {
        Ok(_probe_info) => match mind_flow::engine::gpu::spawn_server(
            &engine_path,
            &models_dir,
            std::time::Duration::from_secs(60),
        )
        .await
        {
            Ok((mut child, port, token, info)) => {
                match mind_flow::engine::remote::RemoteEngine::new(port, token, info.clone()) {
                    Ok(engine) => {
                        tracing::info!("已切换 CUDA 引擎：{}", info.device);
                        state.engine.swap(Box::new(engine));
                    }
                    Err(error) => tracing::warn!("创建 GPU 客户端失败：{error}"),
                }
                let _ = shutdown.changed().await;
                let _ = child.kill().await;
            }
            Err(error) => {
                let message = format!("CUDA 引擎启动失败（{error}），继续用 CPU");
                tracing::warn!("{message}");
                *state.engine_reason.lock().unwrap() = message;
                state.broadcast(Event::Status(state.status()));
            }
        },
        Err(error) => {
            let message = format!("GPU 不可用（{error}），继续用 CPU");
            tracing::info!("{message}");
            *state.engine_reason.lock().unwrap() = message;
            if state.config.lock().unwrap().inference == Inference::Cuda {
                state.broadcast(Event::Error {
                    code: "cuda_required".into(),
                    message: format!("--inference cuda 要求 GPU 可用：{error}"),
                });
            }
            state.broadcast(Event::Status(state.status()));
        }
    }
}

/// 端口被占用时依次尝试后面的端口。
async fn bind_listener(host: &str, port: u16) -> Result<tokio::net::TcpListener> {
    let mut last_error = None;
    for offset in 0..20u16 {
        let candidate = port.saturating_add(offset);
        match tokio::net::TcpListener::bind((host, candidate)).await {
            Ok(listener) => {
                if offset > 0 {
                    tracing::warn!("端口 {port} 被占用，改用 {candidate}");
                }
                return Ok(listener);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("无法绑定端口"))
        .context("端口都被占用"))
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = result {
        tracing::warn!("打开浏览器失败（请手动访问 {url}）：{error}");
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到退出信号，正在收尾…");
}
