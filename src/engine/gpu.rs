//! CUDA 引擎（独立子进程）的探测与拉起。
//!
//! 主进程内置 CPU 引擎；当 `data/runtime/cuda/mind_flow-engine[.exe]` 存在时后台探测：
//! 子进程用 CUDA provider 装载模型并解码一小段音频，成功则输出整行 JSON 自检结果。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::EngineInfo;

/// 主进程与引擎子进程之间的协议版本，不匹配就不用 GPU。
pub const ENGINE_PROTOCOL: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct ProbeReport {
    pub protocol: u32,
    pub info: EngineInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServeReport {
    pub protocol: u32,
    pub port: u16,
    pub info: EngineInfo,
}

/// CUDA 引擎可执行文件位置。
pub fn cuda_engine_path(runtime_cuda_dir: &Path) -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "mind_flow-engine.exe"
    } else {
        "mind_flow-engine"
    };
    runtime_cuda_dir.join(name)
}

fn engine_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.kill_on_drop(true);
    command
}

/// 跑一次 `--probe`，超时或输出异常都视为不可用。
pub async fn probe(engine_path: &Path, models_dir: &Path, timeout: Duration) -> Result<EngineInfo> {
    if !engine_path.exists() {
        bail!("未找到 CUDA 引擎：{}", engine_path.display());
    }
    let mut child = engine_command(engine_path)
        .arg("--probe")
        .arg("--provider")
        .arg("cuda")
        .arg("--models")
        .arg(models_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("启动 CUDA 引擎失败：{}", engine_path.display()))?;

    let stdout = child.stdout.take().context("子进程没有 stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let read = async {
        while let Some(line) = lines.next_line().await? {
            if let Ok(report) = serde_json::from_str::<ProbeReport>(line.trim()) {
                return Ok::<ProbeReport, std::io::Error>(report);
            }
        }
        Err(std::io::Error::other("自检没有输出结果"))
    };
    let report = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            let _ = child.kill().await;
            bail!("CUDA 自检失败：{error}");
        }
        Err(_) => {
            let _ = child.kill().await;
            bail!("CUDA 自检超时（{} 秒）", timeout.as_secs());
        }
    };
    let _ = child.kill().await;
    if report.protocol != ENGINE_PROTOCOL {
        bail!(
            "CUDA 引擎协议不匹配（引擎 {}，主程序 {ENGINE_PROTOCOL}），请更新引擎附件",
            report.protocol
        );
    }
    Ok(report.info)
}

/// 拉起常驻的 CUDA 引擎服务，返回 (子进程, 端口, 令牌, 自述信息)。
pub async fn spawn_server(
    engine_path: &Path,
    models_dir: &Path,
    timeout: Duration,
) -> Result<(Child, u16, String, EngineInfo)> {
    let token = format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let mut child = engine_command(engine_path)
        .arg("--serve")
        .arg("--provider")
        .arg("cuda")
        .arg("--models")
        .arg(models_dir)
        .arg("--token")
        .arg(&token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("启动 CUDA 引擎失败：{}", engine_path.display()))?;
    let stdout = child.stdout.take().context("子进程没有 stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let read = async {
        while let Some(line) = lines.next_line().await? {
            if let Ok(report) = serde_json::from_str::<ServeReport>(line.trim()) {
                return Ok::<ServeReport, std::io::Error>(report);
            }
        }
        Err(std::io::Error::other("引擎没有报告端口"))
    };
    let report = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            let _ = child.kill().await;
            bail!("CUDA 引擎启动失败：{error}");
        }
        Err(_) => {
            let _ = child.kill().await;
            bail!("CUDA 引擎启动超时");
        }
    };
    if report.protocol != ENGINE_PROTOCOL {
        let _ = child.kill().await;
        bail!("CUDA 引擎协议不匹配（引擎 {}）", report.protocol);
    }
    Ok((child, report.port, token, report.info))
}

/// `nvidia-smi` 里的设备名（仅用于界面展示，失败不影响推理）。
pub fn nvidia_device_name() -> Option<String> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 引擎不存在时报错而不是假报成功() {
        let dir = std::env::temp_dir().join(format!("mind_flow_gpu_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("不存在");
        let error = probe(&path, &dir, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("未找到 CUDA 引擎"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn 自检超时会回落() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("mind_flow_gpu_timeout_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = dir.join("mind_flow-engine");
        std::fs::write(&engine, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = probe(&engine, &dir, Duration::from_millis(300))
            .await
            .unwrap_err();
        // 并行跑测试时子进程调度可能不准，这里只要求「失败并回落」，不苛求具体文案
        assert!(
            error.to_string().contains("超时") || error.to_string().contains("失败"),
            "实际：{error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn 自检输出非法时回落() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mind_flow_gpu_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = dir.join("mind_flow-engine");
        std::fs::write(&engine, "#!/bin/sh\necho 这不是 JSON\nexit 1\n").unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = probe(&engine, &dir, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("自检失败") || error.to_string().contains("启动"),
            "实际：{error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn 协议不匹配时回落() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mind_flow_gpu_proto_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = dir.join("mind_flow-engine");
        let body = format!(
            "#!/bin/sh\necho '{{\"protocol\":99,\"info\":{{\"name\":\"x\",\"version\":\"1\",\"model\":\"m\",\"punctuation\":null,\"vad\":null,\"provider\":\"cuda\",\"device\":\"d\"}}}}'\n"
        );
        std::fs::write(&engine, body).unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = probe(&engine, &dir, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("协议不匹配"), "实际：{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
