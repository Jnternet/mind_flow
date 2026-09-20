//! 独立引擎进程：给主程序提供 CUDA 推理（也可用于 `--probe` 自检）。

// 未启用 sherpa 特性时，下面这些只在真实引擎路径用到的定义会「未被使用」
#![cfg_attr(
    not(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda")),
    allow(dead_code, unused_imports)
)]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;

const ENGINE_PROTOCOL: u32 = 1;

#[derive(Parser, Debug)]
#[command(name = "mind_flow-engine", version, about = "GPU 识别引擎（独立进程）")]
struct Cli {
    /// 自检：装载模型 + 解码一小段音频后输出一行 JSON 并退出
    #[arg(long)]
    probe: bool,
    /// 常驻服务模式
    #[arg(long)]
    serve: bool,
    /// 推理 provider：cpu / cuda
    #[arg(long, default_value = "cuda")]
    provider: String,
    /// 模型目录
    #[arg(long)]
    models: PathBuf,
    /// 监听端口（0 表示随机）
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// 鉴权令牌
    #[arg(long, default_value = "")]
    token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if !cli.probe && !cli.serve {
        bail!("请指定 --probe 或 --serve");
    }
    let paths = mind_flow::models::ModelPaths::resolve(&cli.models);
    if !paths.is_ready() {
        bail!("模型不完整：{}", paths.asr_model.display());
    }
    let device = if cli.provider == "cuda" {
        mind_flow::engine::gpu::nvidia_device_name().unwrap_or_else(|| "CUDA 设备".to_string())
    } else {
        "CPU".to_string()
    };
    let label = paths
        .asr_model
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "paraformer-zh".to_string());
    let threads = std::thread::available_parallelism()
        .map(|value| value.get().min(8) as i32)
        .unwrap_or(4);

    #[cfg(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda"))]
    {
        use mind_flow::engine::Engine;
        use mind_flow::engine::sherpa::SherpaEngine;
        let engine = SherpaEngine::load(&paths, &cli.provider, threads, device.clone(), label)
            .context("装载引擎失败（CUDA/cuDNN 缺失或版本不匹配？）")?;
        if cli.probe {
            // 用 0.5 秒正弦做一次真实解码，确认 provider 真的能跑
            let samples: Vec<i16> = (0..8000)
                .map(|i| ((i as f32 * 0.1).sin() * 8000.0) as i16)
                .collect();
            engine
                .recognize(&samples)
                .context("自检解码失败（provider 不可用）")?;
            let report = serde_json::json!({
                "protocol": ENGINE_PROTOCOL,
                "info": engine.info(),
            });
            println!("{report}");
            return Ok(());
        }
        return serve(engine, cli).await;
    }
    #[cfg(not(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda")))]
    {
        let _ = (paths, cli, device, label, threads);
        bail!("本二进制编译时未启用 sherpa 特性");
    }
}

async fn serve(engine: impl mind_flow::engine::Engine + 'static, cli: Cli) -> Result<()> {
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};

    let engine: Arc<dyn mind_flow::engine::Engine> = Arc::new(engine);
    let info = engine.info();
    let token = cli.token.clone();

    struct Server {
        engine: std::sync::Mutex<Arc<dyn mind_flow::engine::Engine>>,
        token: String,
    }
    let state = Arc::new(Server {
        engine: std::sync::Mutex::new(engine),
        token,
    });

    let app = Router::new()
        .route(
            "/recognize",
            post(
                |State(state): State<Arc<Server>>, headers: HeaderMap, body: Bytes| async move {
                    let provided = headers
                        .get("x-mind-flow-token")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("");
                    if provided != state.token {
                        return (StatusCode::FORBIDDEN, "token 不匹配").into_response();
                    }
                    let rate = headers
                        .get("x-sample-rate")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(16000);
                    let pcm = mind_flow::audio::bytes_to_i16_le(&body);
                    let pcm = if rate == mind_flow::audio::TARGET_RATE {
                        pcm
                    } else {
                        mind_flow::audio::resample_i16(&pcm, rate, mind_flow::audio::TARGET_RATE)
                    };
                    let engine = state.engine.lock().unwrap().clone();
                    match engine.recognize(&pcm) {
                        Ok(sentences) => Json(serde_json::json!({
                            "sentences": sentences,
                            "info": engine.info(),
                        }))
                        .into_response(),
                        Err(error) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": error.to_string()})),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cli.port)).await?;
    let port = listener.local_addr()?.port();
    println!(
        "{}",
        serde_json::json!({
            "protocol": ENGINE_PROTOCOL,
            "port": port,
            "info": info,
        })
    );
    // 父进程退出（stdin EOF）时自己也退出
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buffer = [0u8; 64];
        let mut stdin = tokio::io::stdin();
        loop {
            match stdin.read(&mut buffer).await {
                Ok(0) | Err(_) => std::process::exit(0),
                Ok(_) => {}
            }
        }
    });
    axum::serve(listener, app).await?;
    Ok(())
}
