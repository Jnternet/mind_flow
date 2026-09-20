//! 音频工具：PCM 转换、WAV 增量写入、重采样。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

/// 识别与落盘统一使用的采样率。
pub const TARGET_RATE: u32 = 16_000;

/// 小端 Int16 字节流 → i16 采样。
pub fn bytes_to_i16_le(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// i16 采样 → 小端字节流。
pub fn i16_to_bytes_le(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// i16 采样 → f32（[-1, 1]），喂给识别引擎用。
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|s| *s as f32 / 32768.0).collect()
}

/// 时长（毫秒）。
pub fn duration_ms(sample_count: usize, rate: u32) -> u32 {
    if rate == 0 {
        return 0;
    }
    ((sample_count as u64 * 1000) / rate as u64) as u32
}

/// 生成静音。
pub fn silence(ms: u32, rate: u32) -> Vec<i16> {
    vec![0i16; (ms as u64 * rate as u64 / 1000) as usize]
}

/// 增量写入的 16k 单声道 WAV：每追加一段就重写一次头部，保证浏览器能直接播放已有部分。
pub struct WavWriter {
    file: File,
    data_bytes: u64,
}

impl WavWriter {
    /// 打开（或创建）文件。已存在且是 WAV 时按现有数据长度续写。
    pub fn open(path: &Path, rate: u32) -> Result<Self> {
        if path.exists() {
            let data_bytes = existing_data_bytes(path).unwrap_or(0);
            let mut file = File::options().read(true).write(true).open(path)?;
            file.seek(SeekFrom::Start(data_bytes + 44))?;
            let mut writer = WavWriter { file, data_bytes };
            writer.write_header(rate)?;
            return Ok(writer);
        }
        let file = File::create(path).with_context(|| format!("创建 {} 失败", path.display()))?;
        let mut writer = WavWriter {
            file,
            data_bytes: 0,
        };
        writer.write_header(rate)?;
        Ok(writer)
    }

    fn write_header(&mut self, rate: u32) -> Result<()> {
        let header = wav_header(rate, 1, self.data_bytes);
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header)?;
        self.file.seek(SeekFrom::Start(44 + self.data_bytes))?;
        Ok(())
    }

    pub fn append_i16(&mut self, samples: &[i16], rate: u32) -> Result<()> {
        self.file.seek(SeekFrom::Start(44 + self.data_bytes))?;
        self.file.write_all(&i16_to_bytes_le(samples))?;
        self.data_bytes += (samples.len() * 2) as u64;
        self.file.flush()?;
        self.write_header(rate)?;
        Ok(())
    }

    pub fn data_bytes(&self) -> u64 {
        self.data_bytes
    }
}

/// 44 字节 WAV 头（PCM，小端）。
pub fn wav_header(rate: u32, channels: u16, data_bytes: u64) -> Vec<u8> {
    let bits = 16u16;
    let byte_rate = rate * channels as u32 * (bits / 8) as u32;
    let block_align = channels * bits / 8;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&bits.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    header
}

fn existing_data_bytes(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 44];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    Some(u32::from_le_bytes([header[40], header[41], header[42], header[43]]) as u64)
}

/// 写一个完整的 WAV（测试与导出用）。
pub fn write_wav_i16(path: &Path, samples: &[i16], rate: u32) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(&wav_header(rate, 1, (samples.len() * 2) as u64))?;
    file.write_all(&i16_to_bytes_le(samples))?;
    file.flush()?;
    Ok(())
}

/// 读 WAV（只支持 16bit PCM 单/多声道，多声道取第一声道），返回 (采样, 采样率)。
pub fn read_wav_mono16(path: &Path) -> Result<(Vec<i16>, u32)> {
    let mut file = File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    parse_wav_mono16(&bytes)
}

/// 从内存解析 WAV（只支持 16bit PCM，多声道取第一声道）。
pub fn parse_wav_mono16(bytes: &[u8]) -> Result<(Vec<i16>, u32)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" {
        bail!("不是合法的 WAV 文件");
    }
    let mut pos = 12usize;
    let mut rate = TARGET_RATE;
    let mut channels = 1u16;
    let mut bits = 16u16;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body_start = pos + 8;
        let body_end = (body_start + size).min(bytes.len());
        match id {
            b"fmt " if size >= 16 => {
                channels = u16::from_le_bytes([bytes[body_start + 2], bytes[body_start + 3]]);
                rate = u32::from_le_bytes([
                    bytes[body_start + 4],
                    bytes[body_start + 5],
                    bytes[body_start + 6],
                    bytes[body_start + 7],
                ]);
                bits = u16::from_le_bytes([bytes[body_start + 14], bytes[body_start + 15]]);
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        pos = body_end + (size % 2);
    }
    let data = data.context("WAV 缺少 data 块")?;
    if bits != 16 {
        bail!("只支持 16bit PCM WAV（当前 {bits}bit）");
    }
    let samples: Vec<i16> = if channels <= 1 {
        bytes_to_i16_le(data)
    } else {
        bytes_to_i16_le(data)
            .chunks(channels as usize)
            .map(|frame| frame[0])
            .collect()
    };
    Ok((samples, rate))
}

/// 重采样：先做抗混叠低通（加窗 sinc），再线性插值取点。
/// 因为只在整段录音结束后调用，不涉及流式状态。
pub fn resample_i16(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if input.is_empty() || from == 0 || to == 0 || from == to {
        return input.to_vec();
    }
    let samples: Vec<f32> = i16_to_f32(input);
    let filtered = if to < from {
        low_pass(&samples, to as f32 / from as f32)
    } else {
        samples
    };
    let ratio = from as f64 / to as f64;
    let out_len = ((filtered.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let pos = index as f64 * ratio;
        let left = pos.floor() as usize;
        let frac = (pos - left as f64) as f32;
        let a = filtered.get(left).copied().unwrap_or(0.0);
        let b = filtered.get(left + 1).copied().unwrap_or(a);
        let value = a + (b - a) * frac;
        out.push((value.clamp(-1.0, 1.0) * 32767.0).round() as i16);
    }
    out
}

/// 截止频率取目标奈奎斯特（即 0.5 × 降采样比），96 抽头汉宁窗 sinc。
fn low_pass(samples: &[f32], cutoff_ratio: f32) -> Vec<f32> {
    const TAPS: usize = 96;
    let cutoff = (cutoff_ratio * 0.5).clamp(0.01, 0.49);
    let mut kernel = vec![0f32; TAPS];
    let mut sum = 0f32;
    let center = (TAPS - 1) as f32 / 2.0;
    for (i, value) in kernel.iter_mut().enumerate() {
        let x = i as f32 - center;
        let sinc = if x.abs() < 1e-6 {
            2.0 * cutoff
        } else {
            (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
        };
        let window = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (TAPS - 1) as f32).cos();
        *value = sinc * window;
        sum += *value;
    }
    for value in kernel.iter_mut() {
        *value /= sum;
    }
    let mut out = vec![0f32; samples.len()];
    for (i, _) in samples.iter().enumerate() {
        let mut acc = 0f32;
        for (k, weight) in kernel.iter().enumerate() {
            let index = i as isize - k as isize + center as isize;
            let sample = if index < 0 {
                0.0
            } else {
                samples.get(index as usize).copied().unwrap_or(0.0)
            };
            acc += sample * weight;
        }
        out[i] = acc;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 正弦(rate: u32, freq: f32, ms: u32) -> Vec<i16> {
        let count = (rate as u64 * ms as u64 / 1000) as usize;
        (0..count)
            .map(|i| {
                let t = i as f32 / rate as f32;
                ((2.0 * std::f32::consts::PI * freq * t).sin() * 20000.0) as i16
            })
            .collect()
    }

    #[test]
    fn 字节转换往返() {
        let samples = vec![0i16, 1, -1, 32767, -32768, 1234];
        let bytes = i16_to_bytes_le(&samples);
        assert_eq!(bytes_to_i16_le(&bytes), samples);
    }

    #[test]
    fn 时长换算() {
        assert_eq!(duration_ms(16000, 16000), 1000);
        assert_eq!(duration_ms(8000, 16000), 500);
        assert_eq!(duration_ms(0, 16000), 0);
    }

    #[test]
    fn 同采样率不重采样() {
        let input = 正弦(16000, 440.0, 100);
        assert_eq!(resample_i16(&input, 16000, 16000), input);
    }

    #[test]
    fn 降采样长度与幅度正确() {
        let input = 正弦(48000, 1000.0, 500);
        let out = resample_i16(&input, 48000, 16000);
        let expected = input.len() / 3;
        assert!(
            (out.len() as i64 - expected as i64).abs() <= 2,
            "长度 {} 应接近 {expected}",
            out.len()
        );
        let peak = out.iter().map(|s| s.abs()).max().unwrap();
        assert!(peak > 15000, "1kHz 正弦应基本保留，实际峰值 {peak}");
    }

    #[test]
    fn 抗混叠滤掉高频() {
        // 20kHz 在 48k 下合法，但降到 16k 后应被滤掉（否则会折叠成 4kHz 假信号）
        let input = 正弦(48000, 20000.0, 500);
        let out = resample_i16(&input, 48000, 16000);
        let peak = out.iter().map(|s| s.abs()).max().unwrap();
        assert!(peak < 2000, "20kHz 应被抗混叠滤除，实际峰值 {peak}");
    }

    #[test]
    fn wav头与读写() {
        let dir = std::env::temp_dir().join(format!("mind_flow_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        let samples = 正弦(16000, 440.0, 200);
        write_wav_i16(&path, &samples, 16000).unwrap();
        let (read, rate) = read_wav_mono16(&path).unwrap();
        assert_eq!(rate, 16000);
        assert_eq!(read.len(), samples.len());
        assert!((read[100] as i32 - samples[100] as i32).abs() <= 1);
        assert_eq!(existing_data_bytes(&path), Some((samples.len() * 2) as u64));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wav增量追加并更新头() {
        let dir = std::env::temp_dir().join(format!("mind_flow_wav2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.wav");
        let first = 正弦(16000, 440.0, 100);
        let second = 正弦(16000, 440.0, 50);
        let mut writer = WavWriter::open(&path, 16000).unwrap();
        writer.append_i16(&first, 16000).unwrap();
        writer.append_i16(&second, 16000).unwrap();
        drop(writer);
        let (read, _) = read_wav_mono16(&path).unwrap();
        assert_eq!(read.len(), first.len() + second.len());
        // 重新打开续写
        let mut writer = WavWriter::open(&path, 16000).unwrap();
        writer.append_i16(&first, 16000).unwrap();
        drop(writer);
        let (read, _) = read_wav_mono16(&path).unwrap();
        assert_eq!(read.len(), first.len() * 2 + second.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 静音长度() {
        assert_eq!(silence(400, 16000).len(), 6400);
    }
}
