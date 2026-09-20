//! 远端引擎客户端：把 16k PCM POST 给 CUDA 引擎子进程。

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::{Engine, EngineInfo, RawSentence};

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteSentence {
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteResponse {
    pub sentences: Vec<RemoteSentence>,
    pub info: EngineInfo,
}

/// 指向 `mind_flow-engine --serve` 的客户端。
pub struct RemoteEngine {
    client: reqwest::blocking::Client,
    url: String,
    token: String,
    info: EngineInfo,
}

impl RemoteEngine {
    pub fn new(port: u16, token: String, info: EngineInfo) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            client,
            url: format!("http://127.0.0.1:{port}/recognize"),
            token,
            info,
        })
    }
}

impl Engine for RemoteEngine {
    fn recognize(&self, pcm16k: &[i16]) -> Result<Vec<RawSentence>> {
        let body = crate::audio::i16_to_bytes_le(pcm16k);
        let response = self
            .client
            .post(&self.url)
            .header("x-mind-flow-token", &self.token)
            .header("x-sample-rate", crate::audio::TARGET_RATE.to_string())
            .body(body)
            .send()
            .context("连接 GPU 引擎失败")?;
        if !response.status().is_success() {
            bail!("GPU 引擎返回 {}", response.status());
        }
        let parsed: RemoteResponse = response.json().context("解析 GPU 引擎响应失败")?;
        Ok(parsed
            .sentences
            .into_iter()
            .map(|s| RawSentence {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text,
            })
            .collect())
    }

    fn info(&self) -> EngineInfo {
        self.info.clone()
    }
}
