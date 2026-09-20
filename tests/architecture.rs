//! 架构约束测试：把「脚本化前端、运行时零网络、只写数据目录」这些要求固化下来。

use std::fs;
use std::path::{Path, PathBuf};

fn 仓库根目录() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 递归收集某个目录下指定后缀的文件。
fn 收集(dir: &Path, suffixes: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| suffixes.contains(&ext))
                .unwrap_or(false)
            {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn 前端资源齐全且全部内嵌() {
    let root = 仓库根目录().join("web");
    for name in ["index.html", "app.js", "styles.css"] {
        assert!(root.join(name).is_file(), "缺少前端文件 web/{name}");
    }
    // 内嵌资源不允许外链（CDN、字体、远程脚本），保证运行时不联网
    for file in 收集(&root, &["html", "js", "css"]) {
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("http://") && !text.contains("https://"),
            "{} 里不应出现外部链接",
            file.display()
        );
    }
}

#[test]
fn web_lib_只有纯逻辑便于单测() {
    let lib = 仓库根目录().join("web").join("lib");
    let files = 收集(&lib, &["js"]);
    assert!(!files.is_empty(), "web/lib 下应有纯逻辑模块");
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for forbidden in ["document.", "window.", "localStorage"] {
            assert!(
                !text.contains(forbidden),
                "{} 不该碰 {forbidden}（纯逻辑模块才能用 Node 测）",
                file.display()
            );
        }
    }
}

#[test]
fn 运行时网络调用只在模型下载与引擎客户端() {
    let src = 仓库根目录().join("src");
    let allowed = ["models.rs", "remote.rs"];
    for file in 收集(&src, &["rs"]) {
        let text = fs::read_to_string(&file).unwrap();
        if text.contains("reqwest::") {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                allowed.contains(&name.as_str()),
                "{} 出现了网络调用，但只允许出现在 {allowed:?}",
                file.display()
            );
        }
    }
}

#[test]
fn 只在数据目录写文件() {
    let src = 仓库根目录().join("src");
    let allowed = [
        "audio.rs",
        "config.rs",
        "export.rs",
        "gpu.rs",
        "logging.rs",
        "models.rs",
        "session.rs",
    ];
    for file in 收集(&src, &["rs"]) {
        let text = fs::read_to_string(&file).unwrap();
        let writes = ["File::create", "create_dir_all", "fs::write", "write_all"];
        if writes.iter().any(|token| text.contains(token)) {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                allowed.contains(&name.as_str()),
                "{} 在写文件，但只允许出现在 {allowed:?}",
                file.display()
            );
        }
    }
    // 主流程与 HTTP 层不直接落盘，避免绕过会话目录
    for name in ["main.rs", "api.rs"] {
        let text = fs::read_to_string(src.join(name)).unwrap();
        for token in ["File::create", "fs::write"] {
            assert!(
                !text.contains(token),
                "src/{name} 不该直接调用 {token}（应交给 session/audio 模块）"
            );
        }
    }
}

#[test]
fn 默认产物不内嵌引擎二进制() {
    let src = 仓库根目录().join("src");
    for file in 收集(&src, &["rs"]) {
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("include_bytes!"),
            "{} 内嵌了二进制资源；CUDA 引擎应当放在 data/runtime/cuda 由脚本取用",
            file.display()
        );
    }
}

#[test]
fn 识别永远跑在独立线程里() {
    let engine = fs::read_to_string(仓库根目录().join("src/engine/mod.rs")).unwrap();
    assert!(
        engine.contains("std::thread::Builder::new"),
        "识别必须跑在独立 OS 线程里，不能占着 async 运行时"
    );
    let api = fs::read_to_string(仓库根目录().join("src/api.rs")).unwrap();
    assert!(
        !api.contains(".recognize("),
        "HTTP 层不应直接调用识别（要走队列）"
    );
}
