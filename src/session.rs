//! 会话状态：段落 / 句子 / 合并音频 / 增量落盘 / 恢复 / 导出。

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::audio::{TARGET_RATE, WavWriter, bytes_to_i16_le, resample_i16, silence, write_wav_i16};
use crate::config::Paths;
use crate::engine::{EngineInfo, RawSentence};
use crate::export;
use crate::sentences::offset_sentences;

/// 段间插入的静音（毫秒）。
pub const GAP_MS: u32 = 400;
/// 短于这个时长的段落直接丢掉（避免误触）。
pub const MIN_SEGMENT_MS: u32 = 300;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: u32,
    pub start_ms: u32,
    pub end_ms: u32,
    #[serde(default)]
    pub empty: bool,
    #[serde(default)]
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentence {
    pub id: String,
    pub segment_id: u32,
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
    #[serde(default)]
    pub failed: bool,
}

/// 导出的 `session.json` 结构（v1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDoc {
    pub version: u32,
    pub title: String,
    pub created_at: String,
    pub audio: String,
    pub duration_ms: u32,
    pub sample_rate: u32,
    pub channel: u32,
    pub sample_format: String,
    pub engine: EngineInfo,
    pub segments: Vec<Segment>,
    pub sentences: Vec<Sentence>,
}

impl SessionDoc {
    fn new(engine: EngineInfo, title: &str, created_at: DateTime<Local>) -> Self {
        Self {
            version: 1,
            title: title.to_string(),
            created_at: created_at.to_rfc3339(),
            audio: "audio.wav".to_string(),
            duration_ms: 0,
            sample_rate: TARGET_RATE,
            channel: 1,
            sample_format: "s16le".to_string(),
            engine,
            segments: Vec::new(),
            sentences: Vec::new(),
        }
    }
}

/// 正在录音的段落。
struct OpenSegment {
    id: u32,
    start_ms: u32,
    raw_path: PathBuf,
    file: File,
}

/// 段落收尾的结果：交给识别队列的音频 + 段落信息。
pub struct FinishedSegment {
    pub segment: Segment,
    pub pcm16k: Vec<i16>,
}

pub struct ActiveSession {
    pub id: String,
    pub dir: PathBuf,
    pub doc: SessionDoc,
    pub audio_version: u64,
    pub pending_jobs: usize,
    pub recording: bool,
    created_at: DateTime<Local>,
    wav: Option<WavWriter>,
    open: Option<OpenSegment>,
}

impl ActiveSession {
    /// 新建会话（工作目录 `sessions/<时间戳-id>/`）。
    pub fn create(paths: &Paths, engine: EngineInfo, title: &str) -> Result<Self> {
        let now = Local::now();
        let id = format!(
            "{}-{:04x}",
            now.format("%Y-%m-%d_%H%M%S"),
            rand::random::<u16>()
        );
        let dir = paths.sessions_dir().join(&id);
        std::fs::create_dir_all(dir.join("raw"))?;
        std::fs::create_dir_all(dir.join("seg"))?;
        let wav = WavWriter::open(&dir.join("audio.wav"), TARGET_RATE)?;
        let session = Self {
            id,
            dir,
            doc: SessionDoc::new(engine, title, now),
            audio_version: 0,
            pending_jobs: 0,
            recording: false,
            created_at: now,
            wav: Some(wav),
            open: None,
        };
        session.save()?;
        Ok(session)
    }

    /// 恢复未完成的会话；返回需要重新识别的段落（(段号, 16k WAV)）。
    pub fn resume(
        paths: &Paths,
        dir: &Path,
        engine: EngineInfo,
    ) -> Result<(Self, Vec<(u32, PathBuf)>)> {
        let doc_path = dir.join("session.json");
        let bytes = std::fs::read(&doc_path)
            .with_context(|| format!("读取会话失败：{}", doc_path.display()))?;
        let mut doc: SessionDoc = serde_json::from_slice(&bytes)?;
        if doc.engine.name == "stub" && engine.name != "stub" {
            doc.engine = engine;
        }
        let created_at = DateTime::parse_from_rfc3339(&doc.created_at)
            .map(|value| value.with_timezone(&Local))
            .unwrap_or_else(|_| Local::now());
        let wav = WavWriter::open(&dir.join("audio.wav"), TARGET_RATE)?;
        let done_segments: Vec<u32> = doc.sentences.iter().map(|s| s.segment_id).collect();
        let mut recovery = Vec::new();
        for segment in &doc.segments {
            if done_segments.contains(&segment.id) || segment.failed {
                continue;
            }
            let pcm_path = dir.join("seg").join(format!("{}.wav", segment.id));
            if pcm_path.exists() {
                recovery.push((segment.id, pcm_path));
            }
        }
        let id = dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "session".to_string());
        let _ = paths;
        Ok((
            Self {
                id,
                dir: dir.to_path_buf(),
                doc,
                audio_version: 0,
                pending_jobs: 0,
                recording: false,
                created_at,
                wav: Some(wav),
                open: None,
            },
            recovery,
        ))
    }

    /// 找最近一个未完成的会话目录。
    pub fn find_unfinished(paths: &Paths) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(paths.sessions_dir())
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("session.json").is_file())
            .collect();
        candidates.sort();
        candidates.pop()
    }

    fn next_segment_id(&self) -> u32 {
        self.doc
            .segments
            .iter()
            .map(|segment| segment.id)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// 开始录一段：非首段先补一段静音，保证时间轴连贯。
    pub fn begin_segment(&mut self, _sample_rate: u32) -> Result<u32> {
        if let Some(open) = self.open.as_ref() {
            bail!("上一段还没结束（段 {}）", open.id);
        }
        if !self.doc.segments.is_empty() {
            if let Some(wav) = self.wav.as_mut() {
                wav.append_i16(&silence(GAP_MS, TARGET_RATE), TARGET_RATE)?;
                self.doc.duration_ms += GAP_MS;
            }
        }
        let id = self.next_segment_id();
        let raw_path = self.dir.join("raw").join(format!("{id}.pcm"));
        let file = File::create(&raw_path)?;
        self.open = Some(OpenSegment {
            id,
            start_ms: self.doc.duration_ms,
            raw_path,
            file,
        });
        self.recording = true;
        Ok(id)
    }

    /// 追加原始 PCM（设备采样率，16bit 小端）。
    pub fn append_pcm(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(open) = self.open.as_mut() else {
            bail!("当前没有在录的段落");
        };
        open.file.write_all(bytes)?;
        Ok(())
    }

    /// 结束当前段：重采样、并入合并音频、落盘待识别 PCM。
    pub fn end_segment(&mut self, sample_rate: u32) -> Result<Option<FinishedSegment>> {
        let Some(open) = self.open.take() else {
            self.recording = false;
            return Ok(None);
        };
        self.recording = false;
        drop(open.file);
        let raw = std::fs::read(&open.raw_path)?;
        let samples = bytes_to_i16_le(&raw);
        let duration = crate::audio::duration_ms(samples.len(), sample_rate);
        if duration < MIN_SEGMENT_MS {
            let _ = std::fs::remove_file(&open.raw_path);
            return Ok(None);
        }
        let pcm16k = resample_i16(&samples, sample_rate, TARGET_RATE);
        let span = crate::audio::duration_ms(pcm16k.len(), TARGET_RATE);
        let segment = Segment {
            id: open.id,
            start_ms: open.start_ms,
            end_ms: open.start_ms + span,
            empty: false,
            failed: false,
        };
        // 保留一份 16k PCM：崩溃/重启后可以重新识别
        write_wav_i16(
            &self.dir.join("seg").join(format!("{}.wav", open.id)),
            &pcm16k,
            TARGET_RATE,
        )?;
        if let Some(wav) = self.wav.as_mut() {
            wav.append_i16(&pcm16k, TARGET_RATE)?;
        }
        self.doc.duration_ms = segment.end_ms;
        self.doc.segments.push(segment.clone());
        let _ = std::fs::remove_file(&open.raw_path);
        self.audio_version += 1;
        self.save()?;
        Ok(Some(FinishedSegment { segment, pcm16k }))
    }

    /// 识别结果写回：相对时间平移成会话绝对时间。
    pub fn add_sentences(
        &mut self,
        segment_id: u32,
        mut sentences: Vec<RawSentence>,
        engine: EngineInfo,
    ) -> Result<Vec<Sentence>> {
        let Some(segment) = self.doc.segments.iter().find(|s| s.id == segment_id) else {
            bail!("段落 {segment_id} 不存在");
        };
        let offset = segment.start_ms;
        offset_sentences(&mut sentences, offset);
        let mut existing = self
            .doc
            .sentences
            .iter()
            .filter(|s| s.segment_id == segment_id)
            .count();
        let mut added = Vec::new();
        for sentence in sentences {
            if sentence.text.trim().is_empty() {
                continue;
            }
            existing += 1;
            added.push(Sentence {
                id: format!("s-{segment_id}-{existing}"),
                segment_id,
                start_ms: sentence.start_ms,
                end_ms: sentence.end_ms,
                text: sentence.text,
                failed: false,
            });
        }
        let empty = added.is_empty();
        if let Some(segment) = self.doc.segments.iter_mut().find(|s| s.id == segment_id) {
            segment.empty = empty;
        }
        self.doc.engine = engine;
        self.doc.sentences.extend(added.iter().cloned());
        self.save()?;
        Ok(added)
    }

    /// 段落识别失败：标记但不丢音频。
    pub fn mark_segment_failed(&mut self, segment_id: u32) -> Result<()> {
        if let Some(segment) = self.doc.segments.iter_mut().find(|s| s.id == segment_id) {
            segment.failed = true;
        }
        self.save()
    }

    /// 编辑句子文字。
    pub fn update_sentence(&mut self, id: &str, text: &str) -> Result<Sentence> {
        let Some(sentence) = self.doc.sentences.iter_mut().find(|s| s.id == id) else {
            bail!("句子 {id} 不存在");
        };
        sentence.text = text.trim().to_string();
        let updated = sentence.clone();
        self.save()?;
        Ok(updated)
    }

    /// 原子写 `session.json`。
    pub fn save(&self) -> Result<()> {
        let path = self.dir.join("session.json");
        let tmp = self.dir.join("session.json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.doc)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// 导出四个文件并删除工作目录。
    pub fn finalize(&mut self, recordings_dir: &Path, title: &str) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(recordings_dir)?;
        let base = export::unique_base(
            recordings_dir,
            &export::filename_base(self.created_at, title),
        );
        std::fs::write(self.dir.join(".finalizing"), &base)?;

        self.wav = None; // 关掉写入句柄，确保文件内容落盘
        self.doc.title = export::sanitize_title(title);
        self.doc.audio = format!("{base}.wav");

        let wav_target = recordings_dir.join(format!("{base}.wav"));
        std::fs::copy(self.dir.join("audio.wav"), &wav_target)?;
        let txt_target = recordings_dir.join(format!("{base}.txt"));
        let srt_target = recordings_dir.join(format!("{base}.srt"));
        let json_target = recordings_dir.join(format!("{base}.json"));
        write_atomic(&txt_target, export::to_txt(&self.doc).as_bytes())?;
        write_atomic(&srt_target, export::to_srt(&self.doc).as_bytes())?;
        write_atomic(
            &json_target,
            serde_json::to_vec_pretty(&self.doc)?.as_slice(),
        )?;

        std::fs::remove_dir_all(&self.dir)
            .with_context(|| format!("清理会话目录失败：{}", self.dir.display()))?;
        Ok(vec![wav_target, txt_target, json_target, srt_target])
    }

    /// 丢弃当前会话的工作目录。
    pub fn discard(&mut self) -> Result<()> {
        self.wav = None;
        if self.dir.exists() {
            std::fs::remove_dir_all(&self.dir)?;
        }
        Ok(())
    }

    /// 会话里的有效句子数。
    pub fn sentence_count(&self) -> usize {
        self.doc
            .sentences
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .count()
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::read_wav_mono16;

    fn 测试目录(name: &str) -> Paths {
        let dir =
            std::env::temp_dir().join(format!("mind_flow_sess_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Paths {
            data_dir: dir,
            portable: true,
        }
    }

    /// 录一段方波（当成「说话」），返回收尾后的段落。
    fn 录一段(session: &mut ActiveSession, ms: u32, rate: u32) -> FinishedSegment {
        session.begin_segment(rate).unwrap();
        let samples: Vec<i16> = (0..(rate as u64 * ms as u64 / 1000) as usize)
            .map(|i| if i % 2 == 0 { 3000 } else { -3000 })
            .collect();
        session
            .append_pcm(&crate::audio::i16_to_bytes_le(&samples))
            .unwrap();
        session.end_segment(rate).unwrap().unwrap()
    }

    #[test]
    fn 断续两段音频按顺序拼接() {
        let paths = 测试目录("append");
        paths.ensure_dirs().unwrap();
        let mut session = ActiveSession::create(&paths, EngineInfo::default(), "测试").unwrap();
        let first = 录一段(&mut session, 1000, 16000);
        assert_eq!(first.segment.start_ms, 0);
        assert_eq!(first.segment.end_ms, 1000);
        let second = 录一段(&mut session, 500, 48000);
        assert_eq!(second.segment.start_ms, 1400, "第二段前面应有 400ms 静音");
        assert_eq!(second.segment.end_ms, 1900);
        let (samples, rate) = read_wav_mono16(&session.dir.join("audio.wav")).unwrap();
        assert_eq!(rate, 16000);
        assert_eq!(samples.len(), 16000 * 1900 / 1000);
        assert_eq!(session.audio_version, 2);
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn 太短的段落被丢弃() {
        let paths = 测试目录("short");
        paths.ensure_dirs().unwrap();
        let mut session = ActiveSession::create(&paths, EngineInfo::default(), "测试").unwrap();
        session.begin_segment(16000).unwrap();
        session
            .append_pcm(&crate::audio::i16_to_bytes_le(&vec![0i16; 1600]))
            .unwrap();
        assert!(session.end_segment(16000).unwrap().is_none());
        assert!(session.doc.segments.is_empty());
        assert_eq!(session.doc.duration_ms, 0);
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn 句子按段落偏移并累加编号() {
        let paths = 测试目录("sent");
        paths.ensure_dirs().unwrap();
        let mut session = ActiveSession::create(&paths, EngineInfo::default(), "测试").unwrap();
        let finished = 录一段(&mut session, 1000, 16000);
        let added = session
            .add_sentences(
                finished.segment.id,
                vec![RawSentence {
                    start_ms: 100,
                    end_ms: 400,
                    text: "你好。".into(),
                }],
                EngineInfo::default(),
            )
            .unwrap();
        assert_eq!(added[0].id, "s-1-1");
        assert_eq!(added[0].start_ms, 100);
        let more = session
            .add_sentences(
                finished.segment.id,
                vec![RawSentence {
                    start_ms: 500,
                    end_ms: 900,
                    text: "再见。".into(),
                }],
                EngineInfo::default(),
            )
            .unwrap();
        assert_eq!(more[0].id, "s-1-2");
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn 崩溃后可恢复并重跑未识别段落() {
        let paths = 测试目录("resume");
        paths.ensure_dirs().unwrap();
        let dir = {
            let mut session = ActiveSession::create(&paths, EngineInfo::default(), "测试").unwrap();
            let _ = 录一段(&mut session, 800, 16000);
            session.dir.clone()
        };
        // 模拟进程退出：段落已落盘但还没识别
        let (mut resumed, recovery) =
            ActiveSession::resume(&paths, &dir, EngineInfo::default()).unwrap();
        assert_eq!(resumed.doc.segments.len(), 1);
        assert_eq!(recovery.len(), 1, "未识别段落应被重新排队");
        assert_eq!(recovery[0].0, 1);
        let second = 录一段(&mut resumed, 400, 16000);
        assert_eq!(second.segment.start_ms, 1200, "恢复后时间轴接着走");
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn 编辑句子会立刻落盘() {
        let paths = 测试目录("edit");
        paths.ensure_dirs().unwrap();
        let mut session = ActiveSession::create(&paths, EngineInfo::default(), "测试").unwrap();
        let finished = 录一段(&mut session, 600, 16000);
        session
            .add_sentences(
                finished.segment.id,
                vec![RawSentence {
                    start_ms: 0,
                    end_ms: 500,
                    text: "识别错了。".into(),
                }],
                EngineInfo::default(),
            )
            .unwrap();
        session.update_sentence("s-1-1", "改好了。").unwrap();
        let doc: SessionDoc =
            serde_json::from_slice(&std::fs::read(session.dir.join("session.json")).unwrap())
                .unwrap();
        assert_eq!(doc.sentences[0].text, "改好了。");
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn 导出四个文件并清理目录() {
        let paths = 测试目录("finalize");
        paths.ensure_dirs().unwrap();
        let mut session = ActiveSession::create(&paths, EngineInfo::default(), "测试").unwrap();
        let finished = 录一段(&mut session, 1200, 16000);
        session
            .add_sentences(
                finished.segment.id,
                vec![RawSentence {
                    start_ms: 200,
                    end_ms: 900,
                    text: "导出我。".into(),
                }],
                EngineInfo::default(),
            )
            .unwrap();
        let dir = session.dir.clone();
        let files = session.finalize(&paths.recordings_dir(), "第一场").unwrap();
        assert_eq!(files.len(), 4);
        for file in &files {
            assert!(file.exists(), "缺少 {}", file.display());
        }
        assert!(!dir.exists(), "会话工作目录应被清理");
        assert!(
            files[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("第一场")
        );
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&files[2]).unwrap()).unwrap();
        assert_eq!(json["sentences"][0]["text"], "导出我。");
        assert!(json["audio"].as_str().unwrap().ends_with(".wav"));
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }
}
