//! 推给前端的实时事件（WS 下行）。

use serde::Serialize;

use crate::session::Sentence;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status(StatusEvent),
    SegmentClosed {
        segment: crate::session::Segment,
    },
    SentencesAdded {
        sentences: Vec<Sentence>,
        audio_version: u64,
    },
    ModelProgress {
        file: String,
        downloaded: u64,
        total: u64,
        finished: bool,
    },
    EngineChanged {
        provider: String,
        device: String,
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusEvent {
    /// 程序版本：确认「页面 / 二进制」是不是同一版时先看这个。
    pub version: String,
    pub session: Option<String>,
    pub sentences: usize,
    pub segments: usize,
    pub pending_jobs: usize,
    pub audio_version: u64,
    pub provider: String,
    pub device: String,
    pub engine_reason: String,
    pub model_ready: bool,
    pub model_extras_ready: bool,
    pub model_downloading: bool,
    pub data_dir: String,
    pub recordings_dir: String,
    /// 程序实际查找模型的目录（排查「模型明明在却不识别」第一要看这个）。
    pub model_dir: String,
    /// 缺失的必需模型（相对路径）。
    pub model_missing: Vec<String>,
    /// 最近一次失败原因（引擎装载 / 模型下载），修好后自动清空。
    pub last_error: Option<String>,
    pub title: String,
    pub recording: bool,
    pub recorder_client: Option<String>,
}
