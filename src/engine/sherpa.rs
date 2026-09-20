//! 真实识别引擎：Silero VAD 分段 → Paraformer-zh 解码 → CT-Transformer 标点 → 句子时间戳。

use anyhow::{Context, Result, bail};
use sherpa_onnx::{
    OfflineParaformerModelConfig, OfflinePunctuation, OfflinePunctuationConfig,
    OfflinePunctuationModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    SileroVadModelConfig, VadModelConfig, VoiceActivityDetector,
};

use super::{Engine, EngineInfo, RawSentence};
use crate::audio::{TARGET_RATE, duration_ms, i16_to_f32};
use crate::models::ModelPaths;
use crate::sentences::build_sentences;

pub const ENGINE_NAME: &str = "sherpa-onnx";
pub const ENGINE_VERSION: &str = "1.13.8";
/// 单次解码的最长音频（秒）。超长语音按 VAD 静音切成多块，与 FunASR 管线的做法一致。
const MAX_CHUNK_SECONDS: f32 = 30.0;

pub struct SherpaEngine {
    recognizer: OfflineRecognizer,
    punct: Option<OfflinePunctuation>,
    vad_config: Option<VadModelConfig>,
    info: EngineInfo,
}

impl SherpaEngine {
    pub fn load(
        paths: &ModelPaths,
        provider: &str,
        threads: i32,
        device: String,
        model_label: String,
    ) -> Result<Self> {
        if !paths.is_ready() {
            bail!("模型不完整：缺少 {}", paths.asr_model.display());
        }
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.paraformer = OfflineParaformerModelConfig {
            model: Some(paths.asr_model.to_string_lossy().to_string()),
        };
        config.model_config.tokens = Some(paths.tokens.to_string_lossy().to_string());
        config.model_config.num_threads = threads;
        config.model_config.provider = Some(provider.to_string());
        config.model_config.debug = false;
        let recognizer = OfflineRecognizer::create(&config)
            .context("创建识别器失败（模型文件损坏或 provider 不可用）")?;

        let punct = paths.punct_model.as_ref().and_then(|model| {
            let mut punct_config = OfflinePunctuationConfig::default();
            punct_config.model = OfflinePunctuationModelConfig {
                ct_transformer: Some(model.to_string_lossy().to_string()),
                num_threads: threads.clamp(1, 2),
                debug: false,
                provider: Some(provider.to_string()),
            };
            OfflinePunctuation::create(&punct_config)
        });

        let vad_config = paths.vad_model.as_ref().map(|model| VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model.to_string_lossy().to_string()),
                threshold: 0.5,
                min_silence_duration: 0.25,
                min_speech_duration: 0.25,
                window_size: 512,
                max_speech_duration: MAX_CHUNK_SECONDS,
            },
            sample_rate: TARGET_RATE as i32,
            num_threads: 1,
            provider: Some(provider.to_string()),
            debug: false,
            ..Default::default()
        });

        Ok(Self {
            recognizer,
            punct,
            vad_config,
            info: EngineInfo {
                name: ENGINE_NAME.to_string(),
                version: ENGINE_VERSION.to_string(),
                model: model_label,
                punctuation: paths.punct_model.as_ref().map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                }),
                vad: paths.vad_model.as_ref().map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                }),
                provider: provider.to_string(),
                device,
            },
        })
    }

    /// VAD 找语音区间（采样下标，左闭右开）。
    fn speech_regions(&self, samples: &[f32]) -> Vec<(usize, usize)> {
        let Some(config) = self.vad_config.as_ref() else {
            return vec![(0, samples.len())];
        };
        let seconds = samples.len() as f32 / TARGET_RATE as f32;
        let buffer_seconds = (seconds + 10.0).clamp(20.0, 900.0);
        let Some(vad) = VoiceActivityDetector::create(config, buffer_seconds) else {
            return vec![(0, samples.len())];
        };
        for chunk in samples.chunks(512) {
            vad.accept_waveform(chunk);
        }
        let mut regions = Vec::new();
        while !vad.is_empty() {
            if let Some(segment) = vad.front() {
                let start = segment.start().max(0) as usize;
                let end = (start + segment.n().max(0) as usize).min(samples.len());
                if end > start {
                    regions.push((start, end));
                }
            }
            vad.pop();
        }
        regions
    }

    /// 把语音区间合并成不超过 30 秒的解码块。
    pub fn plan_chunks(regions: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let max_samples = (MAX_CHUNK_SECONDS * TARGET_RATE as f32) as usize;
        let mut chunks: Vec<(usize, usize)> = Vec::new();
        for (start, end) in regions {
            match chunks.last_mut() {
                Some(last) if end.saturating_sub(last.0) <= max_samples => {
                    last.1 = *end;
                }
                _ => chunks.push((*start, *end)),
            }
        }
        chunks
    }

    fn decode_chunk(&self, samples: &[f32]) -> Result<(String, Vec<String>, Vec<f32>)> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(TARGET_RATE as i32, samples);
        self.recognizer.decode(&stream);
        let result = stream.get_result().context("解码失败")?;
        Ok((
            result.text,
            result.tokens,
            result.timestamps.unwrap_or_default(),
        ))
    }

    fn punctuate(&self, text: &str) -> String {
        match self.punct.as_ref() {
            Some(punct) => punct
                .add_punctuation(text)
                .unwrap_or_else(|| text.to_string()),
            None => text.to_string(),
        }
    }
}

impl Engine for SherpaEngine {
    fn recognize(&self, pcm16k: &[i16]) -> Result<Vec<RawSentence>> {
        if pcm16k.len() < TARGET_RATE as usize / 10 {
            return Ok(Vec::new());
        }
        let samples = i16_to_f32(pcm16k);
        let total_ms = duration_ms(pcm16k.len(), TARGET_RATE);
        let regions = self.speech_regions(&samples);
        if regions.is_empty() {
            return Ok(Vec::new());
        }
        let trim_start = regions[0].0;
        let trim_end = regions[regions.len() - 1].1;
        let chunks = Self::plan_chunks(&regions);

        let mut plain = String::new();
        let mut tokens: Vec<String> = Vec::new();
        let mut timestamps: Vec<f32> = Vec::new();
        for (start, end) in chunks {
            let slice = &samples[start..end.min(samples.len())];
            if slice.len() < TARGET_RATE as usize / 10 {
                continue;
            }
            let (text, chunk_tokens, chunk_stamps) = self.decode_chunk(slice)?;
            if text.trim().is_empty() {
                continue;
            }
            if !plain.is_empty() {
                let previous = plain.chars().last().unwrap_or(' ');
                let next = text.chars().next().unwrap_or(' ');
                if previous.is_ascii_alphanumeric() && next.is_ascii_alphanumeric() {
                    plain.push(' ');
                }
            }
            plain.push_str(&text);
            let offset = start as f32 / TARGET_RATE as f32;
            let has_timestamps = chunk_stamps.len() == chunk_tokens.len();
            for (index, token) in chunk_tokens.into_iter().enumerate() {
                tokens.push(token);
                timestamps.push(if has_timestamps {
                    chunk_stamps[index] + offset
                } else {
                    f32::NAN
                });
            }
        }
        if plain.trim().is_empty() {
            return Ok(Vec::new());
        }
        // 时间戳没给全就整体回退到按字数分配
        if timestamps.iter().any(|value| !value.is_finite()) {
            timestamps.clear();
        }
        let punctuated = self.punctuate(&plain);
        let span = (
            (trim_start as u64 * 1000 / TARGET_RATE as u64) as u32,
            ((trim_end as u64 * 1000 / TARGET_RATE as u64) as u32).min(total_ms),
        );
        Ok(build_sentences(
            &plain,
            &punctuated,
            &tokens,
            &timestamps,
            span,
        ))
    }

    fn info(&self) -> EngineInfo {
        self.info.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 分块不超过上限() {
        let rate = TARGET_RATE as usize;
        let regions = vec![
            (0, 20 * rate),
            (21 * rate, 45 * rate),
            (46 * rate, 50 * rate),
        ];
        let chunks = SherpaEngine::plan_chunks(&regions);
        assert_eq!(chunks.len(), 2);
        for (start, end) in chunks {
            let seconds = (end - start) as f32 / TARGET_RATE as f32;
            assert!(seconds <= MAX_CHUNK_SECONDS + 0.01, "块长 {seconds}");
        }
    }

    #[test]
    fn 单个区间保持原样() {
        assert_eq!(SherpaEngine::plan_chunks(&[(0, 100)]), vec![(0, 100)]);
    }
}
