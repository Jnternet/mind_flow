//! 日志初始化：同时写终端和 `<数据目录>/mind_flow.log`。
//!
//! Windows 上双击运行看不到终端，出问题时让用户把日志文件发回来即可。

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

struct TeeWriter {
    file: Arc<Mutex<std::fs::File>>,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = Write::write_all(&mut std::io::stdout(), buf);
        let mut file = self.file.lock().expect("日志文件锁");
        Write::write_all(&mut *file, buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = Write::flush(&mut std::io::stdout());
        Write::flush(&mut *self.file.lock().expect("日志文件锁"))
    }
}

/// 初始化全局日志（重复调用只生效一次）。返回日志文件路径。
pub fn init(data_dir: &Path) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("mind_flow.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("打开日志文件失败：{}", path.display()))?;
    let handle = Arc::new(Mutex::new(file));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(move || TeeWriter {
            file: handle.clone(),
        })
        .try_init();
    Ok(path)
}
