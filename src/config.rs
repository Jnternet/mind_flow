//! 数据目录与配置文件。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 推理设备偏好。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Inference {
    /// 探测到可用的 CUDA 引擎就用 GPU，否则用 CPU。
    Auto,
    /// 强制 CPU。
    Cpu,
    /// 强制 CUDA，探测失败直接报错。
    Cuda,
}

impl Default for Inference {
    fn default() -> Self {
        Inference::Auto
    }
}

impl Inference {
    pub fn as_str(self) -> &'static str {
        match self {
            Inference::Auto => "auto",
            Inference::Cpu => "cpu",
            Inference::Cuda => "cuda",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Inference::Auto),
            "cpu" => Some(Inference::Cpu),
            "cuda" => Some(Inference::Cuda),
            _ => None,
        }
    }
}

/// 落盘的配置（`data/config.json`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    pub open_browser: bool,
    pub inference: Inference,
    /// 模型下载源前缀，默认走国内可达的 hf 镜像。
    pub model_base_url: String,
    /// 可选代理，例如 `http://192.168.1.2:7890`。
    pub proxy: Option<String>,
    /// 上一次用的标题，作为下次的默认值。
    pub last_title: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8730,
            open_browser: true,
            inference: Inference::Auto,
            model_base_url: "https://hf-mirror.com".to_string(),
            proxy: None,
            last_title: "语音笔记".to_string(),
        }
    }
}

impl Config {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("config.json");
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = data_dir.join("config.json");
        let bytes = serde_json::to_vec_pretty(self)?;
        let tmp = data_dir.join("config.json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// 便携目录布局：默认在程序同级 `data/`，不可写时回退到系统应用数据目录。
#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn resolve(explicit: Option<&Path>) -> Paths {
        if let Some(dir) = explicit {
            let _ = std::fs::create_dir_all(dir);
            return Paths {
                data_dir: dir.to_path_buf(),
                portable: true,
            };
        }
        let portable = default_portable_dir();
        if let Ok(()) = ensure_writable(&portable) {
            return Paths {
                data_dir: portable,
                portable: true,
            };
        }
        let fallback = system_data_dir();
        let _ = ensure_writable(&fallback);
        Paths {
            data_dir: fallback,
            portable: false,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.data_dir.join("models").join(".download")
    }

    pub fn runtime_cuda_dir(&self) -> PathBuf {
        self.data_dir.join("runtime").join("cuda")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.data_dir.join("sessions")
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("recordings")
    }

    /// 建好所有目录。
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            self.data_dir.clone(),
            self.models_dir(),
            self.downloads_dir(),
            self.sessions_dir(),
            self.recordings_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("创建目录失败：{}", dir.display()))?;
        }
        Ok(())
    }
}

fn default_portable_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data")
}

fn system_data_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("mind_flow");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if cfg!(target_os = "macos") {
            return home
                .join("Library")
                .join("Application Support")
                .join("mind_flow");
        }
        return home.join(".local").join("share").join("mind_flow");
    }
    PathBuf::from("mind_flow-data")
}

fn ensure_writable(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(".write-probe");
    std::fs::write(&probe, b"ok")?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_解析() {
        assert_eq!(Inference::parse("CUDA"), Some(Inference::Cuda));
        assert_eq!(Inference::parse("auto"), Some(Inference::Auto));
        assert_eq!(Inference::parse("gpu"), None);
    }

    #[test]
    fn 配置往返() {
        let dir = std::env::temp_dir().join(format!("mind_flow_cfg_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default();
        config.port = 9001;
        config.inference = Inference::Cpu;
        config.save(&dir).unwrap();
        let loaded = Config::load(&dir);
        assert_eq!(loaded.port, 9001);
        assert_eq!(loaded.inference, Inference::Cpu);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
