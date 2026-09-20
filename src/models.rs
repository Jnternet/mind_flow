//! 模型清单、状态检查与下载（SHA-256 校验 + 断点续传 + 解包）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::config::Config;

/// 模型下载源。`{base}` 会被 `config.model_base_url` 替换。
#[derive(Debug, Clone, Copy)]
pub struct ModelSource {
    pub url: &'static str,
    /// `Some(成员名)` 表示下载的是 tar.bz2 归档，需要从中取出该成员。
    pub member: Option<&'static str>,
}

/// 一个模型文件。
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    /// 相对 `data/models/` 的落盘路径。
    pub rel: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    /// 可选文件：下载失败不阻断（标点/VAD 缺失只是效果差一点）。
    pub optional: bool,
    pub sources: &'static [ModelSource],
}

pub const ASR_DIR: &str = "paraformer-zh-2023-09-14-int8";
pub const PUNCT_DIR: &str = "punct-ct-transformer-zh-en-vocab272727-2024-04-12";

/// 默认模型集合：Paraformer-zh（带 token 时间戳的 2023-09-14 int8 导出）+ 标点 + VAD。
/// 约 320MB。
pub const MODEL_FILES: &[ModelFile] = &[
    ModelFile {
        rel: "paraformer-zh-2023-09-14-int8/model.int8.onnx",
        sha256: "f36a0433bcf096bd6d6f11b80a3ac8bed110bdca632fe0d731df8d1a84475945",
        size: 243_371_218,
        sources: &[
            ModelSource {
                url: "{base}/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14/resolve/main/model.int8.onnx",
                member: None,
            },
            ModelSource {
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14/resolve/main/model.int8.onnx",
                member: None,
            },
        ],
        optional: false,
    },
    ModelFile {
        rel: "paraformer-zh-2023-09-14-int8/tokens.txt",
        sha256: "59aba8873a2ed1e122c25fee421e25f283b63290efbde85c1f01a853d83cb6e6",
        size: 75_756,
        sources: &[
            ModelSource {
                url: "{base}/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14/resolve/main/tokens.txt",
                member: None,
            },
            ModelSource {
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14/resolve/main/tokens.txt",
                member: None,
            },
        ],
        optional: false,
    },
    ModelFile {
        rel: "punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.int8.onnx",
        sha256: "65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1",
        size: 75_519_198,
        optional: true,
        sources: &[
            ModelSource {
                url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2",
                member: Some("model.int8.onnx"),
            },
            ModelSource {
                url: "{base}/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2",
                member: Some("model.int8.onnx"),
            },
        ],
    },
    ModelFile {
        rel: "punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.onnx",
        sha256: "e93593a6dbd69a07f8734ef269dbe861a379755f8d1c8354719432116f2c44bd",
        size: 294_372_519,
        optional: true,
        sources: &[
            ModelSource {
                url: "{base}/csukuangfj/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12/resolve/main/model.onnx",
                member: None,
            },
            ModelSource {
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12/resolve/main/model.onnx",
                member: None,
            },
        ],
    },
    ModelFile {
        rel: "silero_vad.onnx",
        sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
        size: 643_854,
        optional: true,
        sources: &[
            ModelSource {
                url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
                member: None,
            },
            ModelSource {
                url: "{base}/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
                member: None,
            },
        ],
    },
];

/// 解析出来的模型路径。
#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub asr_model: PathBuf,
    pub tokens: PathBuf,
    pub punct_model: Option<PathBuf>,
    pub vad_model: Option<PathBuf>,
}

impl ModelPaths {
    pub fn resolve(models_dir: &Path) -> ModelPaths {
        let asr_model = models_dir.join(ASR_DIR).join("model.int8.onnx");
        let tokens = models_dir.join(ASR_DIR).join("tokens.txt");
        let punct_int8 = models_dir.join(PUNCT_DIR).join("model.int8.onnx");
        let punct_fp32 = models_dir.join(PUNCT_DIR).join("model.onnx");
        let punct_model = if punct_int8.exists() {
            Some(punct_int8)
        } else if punct_fp32.exists() {
            Some(punct_fp32)
        } else {
            None
        };
        let vad = models_dir.join("silero_vad.onnx");
        ModelPaths {
            asr_model,
            tokens,
            punct_model,
            vad_model: vad.exists().then_some(vad),
        }
    }

    /// 识别所需的最小集合是否齐全。
    pub fn is_ready(&self) -> bool {
        self.asr_model.exists() && self.tokens.exists()
    }

    /// 标点与 VAD 是否也在（缺了只是效果差一些）。
    pub fn extras_ready(&self) -> bool {
        self.punct_model.is_some() && self.vad_model.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelStatus {
    pub ready: bool,
    pub extras_ready: bool,
    pub missing: Vec<String>,
    pub total_bytes: u64,
    pub present_bytes: u64,
    pub downloading: bool,
}

pub fn status(models_dir: &Path) -> ModelStatus {
    let mut missing = Vec::new();
    let mut total = 0u64;
    let mut present = 0u64;
    for file in MODEL_FILES {
        total += file.size;
        let path = models_dir.join(file.rel);
        if path.exists() {
            present += path.metadata().map(|m| m.len()).unwrap_or(0).min(file.size);
        } else if !file.optional {
            missing.push(file.rel.to_string());
        }
    }
    let paths = ModelPaths::resolve(models_dir);
    ModelStatus {
        ready: paths.is_ready(),
        extras_ready: paths.extras_ready(),
        missing,
        total_bytes: total,
        present_bytes: present,
        downloading: false,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub file: String,
    pub downloaded: u64,
    pub total: u64,
    pub finished: bool,
}

/// 计算文件 SHA-256（分块读，避免把 240MB 全读进内存）。
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn expanded(url: &str, base: &str) -> String {
    url.replace("{base}", base.trim_end_matches('/'))
}

/// 下载全部模型（已存在且校验通过的会跳过）。
pub async fn download_all(
    models_dir: &Path,
    downloads_dir: &Path,
    config: &Config,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Result<()> {
    let client = build_client(config)?;
    std::fs::create_dir_all(downloads_dir)?;
    for file in MODEL_FILES {
        let target = models_dir.join(file.rel);
        if target.exists() {
            if sha256_file(&target)?.eq_ignore_ascii_case(file.sha256) {
                continue;
            }
            // 文件损坏：删掉重下
            let _ = std::fs::remove_file(&target);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut last_error: Option<anyhow::Error> = None;
        for source in file.sources {
            let url = expanded(source.url, &config.model_base_url);
            match fetch_one(
                &client,
                &url,
                source.member,
                file,
                downloads_dir,
                &mut on_progress,
            )
            .await
            {
                Ok(()) => {
                    on_progress(Progress {
                        file: file.rel.to_string(),
                        downloaded: file.size,
                        total: file.size,
                        finished: true,
                    });
                    last_error = None;
                    break;
                }
                Err(error) => {
                    last_error = Some(error.context(format!("下载源失败：{url}")));
                }
            }
        }
        if let Some(error) = last_error {
            if file.optional {
                // 标点 / VAD 缺失只是效果差一点，不当成致命错误
                tracing::warn!("可选模型 {} 下载失败：{error}", file.rel);
                continue;
            }
            return Err(error);
        }
    }
    Ok(())
}

fn build_client(config: &Config) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent("mind_flow/0.1 (+https://github.com/k2-fsa/sherpa-onnx)")
        .timeout(std::time::Duration::from_secs(30 * 60));
    if let Some(proxy) = config.proxy.as_deref().filter(|p| !p.trim().is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    Ok(builder.build()?)
}

async fn fetch_one(
    client: &reqwest::Client,
    url: &str,
    member: Option<&str>,
    file: &ModelFile,
    downloads_dir: &Path,
    on_progress: &mut (impl FnMut(Progress) + Send),
) -> Result<()> {
    let final_path = downloads_dir.join(Path::new(file.rel).file_name().context("模型文件名异常")?);
    let archive_path = downloads_dir.join(format!(
        "{}.tar.bz2",
        Path::new(file.rel)
            .file_name()
            .context("模型文件名异常")?
            .to_string_lossy()
    ));
    let part_path = if member.is_some() {
        archive_path.clone()
    } else {
        final_path.with_extension("part")
    };

    let expected = if member.is_some() {
        // 归档大小未知，用 Content-Length 或已有进度估算
        0
    } else {
        file.size
    };
    download_to(client, url, &part_path, expected, file.rel, on_progress).await?;

    let body_path = if let Some(member) = member {
        extract_member(&part_path, member, &final_path)?;
        let _ = std::fs::remove_file(&part_path);
        final_path.clone()
    } else {
        part_path.clone()
    };
    let digest = sha256_file(&body_path)?;
    if !digest.eq_ignore_ascii_case(file.sha256) {
        let _ = std::fs::remove_file(&body_path);
        bail!("SHA-256 校验失败（期望 {}，实际 {digest}）", file.sha256);
    }
    let target = PathBuf::from(file.rel);
    let _ = target;
    Ok(())
}

/// 带断点续传的下载：已下载的部分用 Range 续传。
async fn download_to(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    expected_total: u64,
    label: &str,
    on_progress: &mut (impl FnMut(Progress) + Send),
) -> Result<()> {
    // 已经完整下载过（大小一致）就跳过
    if let Ok(meta) = std::fs::metadata(path) {
        if expected_total > 0 && meta.len() == expected_total {
            on_progress(Progress {
                file: label.to_string(),
                downloaded: meta.len(),
                total: expected_total,
                finished: true,
            });
            return Ok(());
        }
    }
    let existing = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut request = client.get(url);
    if existing > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        bail!("HTTP {} （{url}）", response.status());
    }
    let resuming = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let start = if resuming { existing } else { 0 };
    let total = response
        .content_length()
        .map(|len| len + start)
        .filter(|total| *total > 0)
        .unwrap_or(expected_total);

    let mut file = if resuming {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await?
    } else {
        tokio::fs::File::create(path).await?
    };
    let mut downloaded = start;
    on_progress(Progress {
        file: label.to_string(),
        downloaded,
        total,
        finished: false,
    });
    let mut stream = response.bytes_stream();
    let mut last_report = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if last_report.elapsed() > std::time::Duration::from_millis(300) {
            last_report = std::time::Instant::now();
            on_progress(Progress {
                file: label.to_string(),
                downloaded,
                total,
                finished: false,
            });
        }
    }
    file.flush().await?;
    Ok(())
}

/// 从 tar.bz2 里取出指定文件名的成员。
fn extract_member(archive: &Path, member: &str, out: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("打开归档失败：{}", archive.display()))?;
    let decoder = bzip2::read::BzDecoder::new(std::io::BufReader::new(file));
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.file_name().map(|name| name == member).unwrap_or(false) {
            let mut out_file = std::fs::File::create(out)?;
            std::io::copy(&mut entry, &mut out_file)?;
            return Ok(());
        }
    }
    bail!("归档里没有 {member}");
}

/// 共享的模型状态（进度回调要跨任务传）。
pub type SharedProgress = Arc<std::sync::Mutex<Progress>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 地址展开() {
        assert_eq!(
            expanded("{base}/a/b", "https://hf-mirror.com/"),
            "https://hf-mirror.com/a/b"
        );
        assert_eq!(expanded("https://x/y", ""), "https://x/y");
    }

    #[test]
    fn 清单里的相对路径与片段名唯一() {
        let mut rels: Vec<&str> = MODEL_FILES.iter().map(|f| f.rel).collect();
        let count = rels.len();
        rels.sort_unstable();
        rels.dedup();
        assert_eq!(rels.len(), count, "模型清单里不应有重复路径");
        for file in MODEL_FILES {
            assert!(!file.sources.is_empty(), "{} 缺少下载源", file.rel);
            assert_eq!(file.sha256.len(), 64, "{} 的 sha256 长度不对", file.rel);
        }
    }

    #[test]
    fn 状态检查指出缺失文件() {
        let dir = std::env::temp_dir().join(format!("mind_flow_models_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let status = status(&dir);
        assert!(!status.ready);
        let required = MODEL_FILES.iter().filter(|file| !file.optional).count();
        assert_eq!(status.missing.len(), required, "只报必需文件缺失");
        assert!(!ModelPaths::resolve(&dir).is_ready());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
