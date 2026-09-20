//! 落盘文件的命名与三种文本格式（txt / json / srt）。

use std::path::Path;

use chrono::{DateTime, Local};

use crate::session::{Sentence, SessionDoc};

/// Windows / macOS / Linux 都不适合出现在文件名里的字符。
const ILLEGAL: &[char] = &[
    '<', '>', ':', '"', '/', '\\', '|', '?', '*', '\n', '\r', '\t',
];
const MAX_TITLE_CHARS: usize = 40;

/// 标题清洗：去掉非法字符、压缩空白、限长。空标题回落到「语音笔记」。
pub fn sanitize_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| if ILLEGAL.contains(&ch) { ' ' } else { ch })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = collapsed.chars().take(MAX_TITLE_CHARS).collect();
    if trimmed.is_empty() {
        "语音笔记".to_string()
    } else {
        trimmed
    }
}

/// 自动命名：`YYYY-MM-DD_HHMM_标题`。
pub fn filename_base(created_at: DateTime<Local>, title: &str) -> String {
    format!(
        "{}_{}",
        created_at.format("%Y-%m-%d_%H%M"),
        sanitize_title(title)
    )
}

/// 同名时追加 `-2`、`-3`。只要四个文件里有一个重名就换编号。
pub fn unique_base(dir: &Path, base: &str) -> String {
    let taken = |candidate: &str| {
        ["wav", "txt", "json", "srt"]
            .iter()
            .any(|ext| dir.join(format!("{candidate}.{ext}")).exists())
    };
    if !taken(base) {
        return base.to_string();
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", Local::now().timestamp())
}

/// 有效句子（编辑清空的句子不导出）。
pub fn visible_sentences(doc: &SessionDoc) -> Vec<&Sentence> {
    doc.sentences
        .iter()
        .filter(|sentence| !sentence.text.trim().is_empty())
        .collect()
}

/// 纯文本：一句一行。
pub fn to_txt(doc: &SessionDoc) -> String {
    let mut out = String::new();
    for sentence in visible_sentences(doc) {
        out.push_str(sentence.text.trim());
        out.push('\n');
    }
    out
}

/// 标准 SRT 字幕。
pub fn to_srt(doc: &SessionDoc) -> String {
    let mut out = String::new();
    for (index, sentence) in visible_sentences(doc).iter().enumerate() {
        out.push_str(&format!("{}\n", index + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            srt_time(sentence.start_ms),
            srt_time(sentence.end_ms)
        ));
        out.push_str(sentence.text.trim());
        out.push_str("\n\n");
    }
    out
}

/// `HH:MM:SS,mmm`
pub fn srt_time(ms: u32) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// 界面上的时间显示：`mm:ss` 或 `h:mm:ss`。
pub fn clock(ms: u32) -> String {
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineInfo;
    use crate::session::{Sentence, SessionDoc};

    fn 样本() -> SessionDoc {
        SessionDoc {
            version: 1,
            title: "语音笔记".into(),
            created_at: "2026-09-20T16:30:12+08:00".into(),
            audio: "x.wav".into(),
            duration_ms: 5000,
            sample_rate: 16000,
            channel: 1,
            sample_format: "s16le".into(),
            engine: EngineInfo::default(),
            segments: vec![],
            sentences: vec![
                Sentence {
                    id: "s-1-1".into(),
                    segment_id: 1,
                    start_ms: 0,
                    end_ms: 1500,
                    text: "第一句。".into(),
                    failed: false,
                },
                Sentence {
                    id: "s-1-2".into(),
                    segment_id: 1,
                    start_ms: 1500,
                    end_ms: 3000,
                    text: "   ".into(),
                    failed: false,
                },
            ],
        }
    }

    #[test]
    fn 标题清洗() {
        assert_eq!(sanitize_title("  今天 讨论/三件事  "), "今天 讨论 三件事");
        assert_eq!(sanitize_title(""), "语音笔记");
        assert_eq!(sanitize_title("???"), "语音笔记", "问号是非法文件名字符");
        assert_eq!(sanitize_title(&"啊".repeat(100)).chars().count(), 40);
    }

    #[test]
    fn 自动命名带时间() {
        let created = chrono::DateTime::parse_from_rfc3339("2026-09-20T16:30:12+08:00")
            .unwrap()
            .with_timezone(&Local);
        let base = filename_base(created, "语音笔记");
        assert!(base.contains("2026-09-20_1630"), "实际：{base}");
        assert!(base.ends_with("语音笔记"));
    }

    #[test]
    fn 重名追加编号() {
        let dir = std::env::temp_dir().join(format!("mind_flow_name_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_base(&dir, "a"), "a");
        std::fs::write(dir.join("a.json"), b"{}").unwrap();
        assert_eq!(unique_base(&dir, "a"), "a-2");
        std::fs::write(dir.join("a-2.txt"), b"x").unwrap();
        assert_eq!(unique_base(&dir, "a"), "a-3");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 文本与字幕格式() {
        let doc = 样本();
        assert_eq!(to_txt(&doc), "第一句。\n");
        let srt = to_srt(&doc);
        assert!(
            srt.starts_with("1\n00:00:00,000 --> 00:00:01,500\n第一句。\n\n"),
            "{srt}"
        );
        assert_eq!(srt.matches("-->").count(), 1, "空句不导出：{srt}");
    }

    #[test]
    fn 时间格式化() {
        assert_eq!(srt_time(0), "00:00:00,000");
        assert_eq!(srt_time(3_723_456), "01:02:03,456");
        assert_eq!(clock(65_000), "01:05");
        assert_eq!(clock(3_665_000), "1:01:05");
    }
}
