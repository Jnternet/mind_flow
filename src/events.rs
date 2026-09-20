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
    pub title: String,
    pub recording: bool,
    pub recorder_client: Option<String>,
}
