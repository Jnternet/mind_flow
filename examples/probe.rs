//! Spike：验证 Paraformer-zh 在 sherpa-onnx 上的行为（时间戳单位、标点、VAD、RTF）。
//!
//! 用法：
//!   cargo run --features sherpa --example probe -- <模型目录> <wav 文件> [标点模型] [VAD 模型]

use std::time::Instant;

use sherpa_onnx::{
    OfflineParaformerModelConfig, OfflinePunctuation, OfflinePunctuationConfig,
    OfflinePunctuationModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    SileroVadModelConfig, VadModelConfig, VoiceActivityDetector, Wave,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let model_dir = args.next().unwrap_or_else(|| {
        eprintln!("需要参数：<模型目录> <wav 文件> [标点模型] [VAD 模型]");
        std::process::exit(2)
    });
    let wav_path = args.next().expect("需要 wav 路径");
    let punct_model = args.next();
    let vad_model = args.next();

    let wave = Wave::read(&wav_path).expect("读 wav 失败");
    let seconds = wave.num_samples() as f32 / wave.sample_rate() as f32;
    println!(
        "音频：{} Hz / {} 采样 / {:.2} 秒",
        wave.sample_rate(),
        wave.num_samples(),
        seconds
    );

    // ---- 识别 ----
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.paraformer = OfflineParaformerModelConfig {
        model: Some(format!("{model_dir}/model.int8.onnx")),
    };
    config.model_config.tokens = Some(format!("{model_dir}/tokens.txt"));
    config.model_config.num_threads = 4;
    config.model_config.provider = Some("cpu".into());
    config.model_config.debug = false;

    let load_start = Instant::now();
    let recognizer = OfflineRecognizer::create(&config).expect("创建识别器失败");
    println!("模型加载耗时：{:.2?}", load_start.elapsed());

    let stream = recognizer.create_stream();
    stream.accept_waveform(wave.sample_rate(), wave.samples());
    let decode_start = Instant::now();
    recognizer.decode(&stream);
    let decode_time = decode_start.elapsed();
    let result = stream.get_result().expect("识别结果为空");

    println!("文本：{}", result.text);
    println!("token 数：{}", result.tokens.len());
    if let Some(ts) = &result.timestamps {
        println!(
            "时间戳：{} 个，前 12 个：{:?}",
            ts.len(),
            &ts[..ts.len().min(12)]
        );
        if let (Some(first), Some(last)) = (ts.first(), ts.last()) {
            println!("时间戳范围：{first} .. {last}（音频时长 {seconds:.3}s）");
            println!("末值与采样点比值：{:.2}", last / (seconds * 16000.0));
        }
    } else {
        println!("时间戳：无（需要回退 VAD 计时）");
    }
    if let Some(d) = &result.durations {
        println!(
            "时长：{} 个，前 12 个：{:?}",
            d.len(),
            &d[..d.len().min(12)]
        );
    }
    println!(
        "解码耗时：{decode_time:.2?}，RTF = {:.4}",
        decode_time.as_secs_f32() / seconds
    );
    println!(
        "tokens 前 20：{:?}",
        &result.tokens[..result.tokens.len().min(20)]
    );

    // ---- 标点 ----
    if let Some(path) = punct_model {
        let mut pconfig = OfflinePunctuationConfig::default();
        pconfig.model = OfflinePunctuationModelConfig {
            ct_transformer: Some(path),
            num_threads: 2,
            debug: false,
            provider: Some("cpu".into()),
        };
        let started = Instant::now();
        let punct = OfflinePunctuation::create(&pconfig).expect("创建标点模型失败");
        println!("标点模型加载：{:.2?}", started.elapsed());
        let started = Instant::now();
        let text = punct.add_punctuation(&result.text).unwrap_or_default();
        println!("标点后：{text}");
        println!("标点耗时：{:.2?}", started.elapsed());
    }

    // ---- VAD ----
    if let Some(path) = vad_model {
        let vconfig = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(path),
                threshold: 0.5,
                min_silence_duration: 0.25,
                min_speech_duration: 0.25,
                window_size: 512,
                max_speech_duration: 30.0,
            },
            sample_rate: 16000,
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&vconfig, 60.0).expect("创建 VAD 失败");
        let samples = wave.samples();
        for chunk in samples.chunks(512) {
            vad.accept_waveform(chunk);
        }
        let mut segments = Vec::new();
        while !vad.is_empty() {
            if let Some(seg) = vad.front() {
                segments.push((seg.start(), seg.n()));
            }
            vad.pop();
        }
        println!("VAD 语音段：{} 个", segments.len());
        for (start, n) in segments.iter().take(8) {
            println!(
                "  起始 {} 采样（{:.3}s），长度 {} 采样（{:.3}s）",
                start,
                *start as f32 / 16000.0,
                n,
                *n as f32 / 16000.0
            );
        }
    }
}
