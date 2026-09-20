//! 识别引擎抽象 + 后台工作线程 + 测试用桩引擎。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

pub use crate::sentences::RawSentence;

#[cfg(any(feature = "sherpa", feature = "sherpa-shared", feature = "sherpa-cuda"))]
pub mod sherpa;

pub mod gpu;
pub mod remote;

/// 引擎自述信息（会写进导出的 json）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
    pub model: String,
    pub punctuation: Option<String>,
    pub vad: Option<String>,
    pub provider: String,
    pub device: String,
}

impl Default for EngineInfo {
    fn default() -> Self {
        Self {
            name: "stub".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            model: "stub".into(),
            punctuation: None,
            vad: None,
            provider: "cpu".into(),
            device: "测试桩".into(),
        }
    }
}

/// 识别引擎：把 16k 单声道 PCM 变成带时间戳的句子。
/// 需要 Send + Sync：引擎既在工作线程里跑，也要能通过 Arc 共享给 HTTP 服务。
pub trait Engine: Send + Sync {
    fn recognize(&self, pcm16k: &[i16]) -> Result<Vec<RawSentence>>;
    fn info(&self) -> EngineInfo;
}

/// 测试用桩引擎：结果稳定可预测，方便端到端断言。
pub struct StubEngine {
    info: EngineInfo,
}

impl StubEngine {
    pub fn new() -> Self {
        Self {
            info: EngineInfo::default(),
        }
    }
}

impl Default for StubEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for StubEngine {
    fn recognize(&self, pcm16k: &[i16]) -> Result<Vec<RawSentence>> {
        let total_ms = crate::audio::duration_ms(pcm16k.len(), crate::audio::TARGET_RATE).max(1);
        // 用音频内容做个小指纹，保证同一段音频结果稳定、不同段落可区分
        let fingerprint: u32 = pcm16k
            .iter()
            .step_by(97)
            .fold(0u32, |acc, s| acc.wrapping_mul(31).wrapping_add(*s as u32));
        let mut sentences = Vec::new();
        let mut cursor = 0u32;
        let mut index = 1;
        while cursor < total_ms {
            let piece = (1400 + (fingerprint % 7) * 50).min(total_ms - cursor);
            sentences.push(RawSentence {
                start_ms: cursor,
                end_ms: cursor + piece,
                text: format!("第{index}句测试文本（桩 {:04x}）。", fingerprint & 0xffff),
            });
            cursor += piece;
            index += 1;
        }
        Ok(sentences)
    }

    fn info(&self) -> EngineInfo {
        self.info.clone()
    }
}

enum WorkerMessage {
    Job { segment_id: u32, pcm: Vec<i16> },
    Swap(Box<dyn Engine>),
    Stop,
}

#[derive(Debug)]
pub enum EngineEvent {
    JobDone {
        segment_id: u32,
        sentences: Vec<RawSentence>,
        elapsed_ms: u64,
        info: EngineInfo,
    },
    JobFailed {
        segment_id: u32,
        error: String,
    },
    EngineChanged {
        info: EngineInfo,
        reason: String,
    },
}

/// 交给上层（axum 状态）用的句柄。
pub struct EngineService {
    tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl EngineService {
    /// 启动工作线程；队列里的任务在引擎就绪前会排队等待。
    pub fn spawn(
        events: mpsc::UnboundedSender<EngineEvent>,
        initial: Option<Box<dyn Engine>>,
        queued: Vec<(u32, Vec<i16>)>,
    ) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<WorkerMessage>();
        std::thread::Builder::new()
            .name("mind_flow-engine".into())
            .spawn(move || {
                let mut engine = initial;
                let mut queue: VecDeque<(u32, Vec<i16>)> = queued.into_iter().collect();
                if let Some(engine) = engine.as_ref() {
                    let _ = events.send(EngineEvent::EngineChanged {
                        info: engine.info(),
                        reason: "启动".into(),
                    });
                }
                while let Some(message) = rx.blocking_recv() {
                    match message {
                        WorkerMessage::Job { segment_id, pcm } => {
                            queue.push_back((segment_id, pcm));
                        }
                        WorkerMessage::Swap(new_engine) => {
                            let info = new_engine.info();
                            engine = Some(new_engine);
                            let _ = events.send(EngineEvent::EngineChanged {
                                info,
                                reason: "引擎切换".into(),
                            });
                        }
                        WorkerMessage::Stop => break,
                    }
                    // 有引擎就尽量把队列跑空。
                    // 注意：引擎没就绪时不能 pop，否则会把排队的任务丢掉。
                    while let Some(active) = engine.as_ref() {
                        let Some((segment_id, pcm)) = queue.pop_front() else {
                            break;
                        };
                        let started = Instant::now();
                        match active.recognize(&pcm) {
                            Ok(sentences) => {
                                let _ = events.send(EngineEvent::JobDone {
                                    segment_id,
                                    sentences,
                                    elapsed_ms: started.elapsed().as_millis() as u64,
                                    info: active.info(),
                                });
                            }
                            Err(error) => {
                                let _ = events.send(EngineEvent::JobFailed {
                                    segment_id,
                                    error: error.to_string(),
                                });
                            }
                        }
                    }
                }
            })
            .expect("启动识别线程失败");
        Arc::new(Self { tx })
    }

    pub fn submit(&self, segment_id: u32, pcm: Vec<i16>) {
        let _ = self.tx.send(WorkerMessage::Job { segment_id, pcm });
    }

    /// 热切换引擎（GPU 探测成功后调用）。
    pub fn swap(&self, engine: Box<dyn Engine>) {
        let _ = self.tx.send(WorkerMessage::Swap(engine));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(WorkerMessage::Stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 桩引擎结果稳定且带时间戳() {
        let engine = StubEngine::new();
        let pcm = vec![0i16; 16000 * 3];
        let first = engine.recognize(&pcm).unwrap();
        let second = engine.recognize(&pcm).unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());
        assert!(first[0].text.starts_with("第1句"));
        assert_eq!(first[0].start_ms, 0);
        assert!(first.last().unwrap().end_ms >= 2900);
    }

    #[test]
    fn 工作线程在引擎就绪后处理排队任务() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let service = EngineService::spawn(tx, None, vec![(7, vec![1i16; 16000])]);
        // 引擎还没就绪时提交的任务会排队
        service.submit(8, vec![2i16; 16000]);
        service.swap(Box::new(StubEngine::new()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let done = runtime.block_on(async {
            let mut ids = Vec::new();
            while ids.len() < 2 {
                match rx.recv().await.unwrap() {
                    EngineEvent::JobDone { segment_id, .. } => ids.push(segment_id),
                    EngineEvent::EngineChanged { .. } => {}
                    other => panic!("意外事件 {other:?}"),
                }
            }
            ids
        });
        assert_eq!(done, vec![7, 8]);
        service.stop();
    }
}
