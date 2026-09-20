//! 句子切分与时间戳对齐（纯逻辑，便于单测）。
//!
//! 输入：识别出的 token 序列 + 每个 token 的起始时间（秒）+ 标点模型输出的整段文本；
//! 输出：一句一行、带起止毫秒（相对本段音频起点）的句子。

use serde::{Deserialize, Serialize};

/// 识别出的句子（时间相对本段音频）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSentence {
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
}

/// 强断句标点：句末。
const STRONG: &[char] = &['。', '！', '？', '!', '?', '…', '；', ';', '\n'];
/// 弱断句标点：分句；只有当当前片段够长才断。
const WEAK: &[char] = &['，', ',', '、', '：', ':'];
/// 只有片段的有效字数达到这个值，才在弱标点处断句（避免「好的，」被切成独立一句）。
const WEAK_MIN_CHARS: usize = 8;
/// 短于这个字数的片段并进上一句。
const MIN_PIECE_CHARS: usize = 2;
/// 最后一个 token 之后预留的时长。
const TAIL_MS: u32 = 220;
/// 单个句子的最短时长。
const MIN_SENTENCE_MS: u32 = 120;

fn is_punct(ch: char) -> bool {
    STRONG.contains(&ch) || WEAK.contains(&ch)
}

/// 把 token 拼成文本，并记录每个字符来自哪个 token。
/// 处理 `@@` 续接标记（BPE）：`es@@` + `day` → `esday`。
fn merge_tokens(tokens: &[String]) -> (String, Vec<usize>) {
    let mut text = String::new();
    let mut owners = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let raw = token.trim_end_matches("@@");
        let piece = raw.trim_end_matches('\u{2581}'); // 部分导出用 ▁ 表示词首
        for ch in piece.chars() {
            text.push(ch);
            owners.push(index);
        }
    }
    (text, owners)
}

/// 每字符对应的 token 下标；token 数与字符数对不上时按比例映射。
fn char_token_map(plain: &str, tokens: &[String]) -> Vec<usize> {
    let (merged, owners) = merge_tokens(tokens);
    let plain_chars: Vec<char> = plain.chars().collect();
    if merged == plain && owners.len() == plain_chars.len() {
        return owners;
    }
    if tokens.is_empty() || plain_chars.is_empty() {
        return vec![0; plain_chars.len()];
    }
    (0..plain_chars.len())
        .map(|i| (i * tokens.len() / plain_chars.len()).min(tokens.len() - 1))
        .collect()
}

/// 把一个字符切成片段：强标点必断，弱标点只在片段够长时断。
fn split_pieces(text: &str) -> Vec<(String, usize, usize)> {
    // (片段文本, 起始字符下标, 结束字符下标（含）)
    let chars: Vec<char> = text.chars().collect();
    let mut pieces: Vec<(String, usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        let strong = STRONG.contains(&ch);
        let weak = WEAK.contains(&ch);
        let visible = chars[start..=index]
            .iter()
            .filter(|c| !is_punct(**c))
            .count();
        if strong || (weak && visible >= WEAK_MIN_CHARS) {
            let piece: String = chars[start..=index].iter().collect();
            pieces.push((piece, start, index));
            start = index + 1;
        }
        index += 1;
    }
    if start < chars.len() {
        let piece: String = chars[start..].iter().collect();
        pieces.push((piece, start, chars.len() - 1));
    }

    // 过短的片段并进上一句
    let mut merged: Vec<(String, usize, usize)> = Vec::new();
    for (piece, from, to) in pieces {
        let visible = piece.chars().filter(|c| !is_punct(*c)).count();
        match merged.last_mut() {
            Some(last) if visible < MIN_PIECE_CHARS => {
                last.0.push_str(&piece);
                last.2 = to;
            }
            _ => merged.push((piece, from, to)),
        }
    }
    merged
}

/// 对齐标点文本与原始文本：返回标点文本每个字符对应的「原始文本字符下标」。
/// 标点模型只会插入标点，不会改字，所以遇到标点就认为它是插入的。
fn align(plain: &str, punctuated: &str) -> Vec<Option<usize>> {
    let plain_chars: Vec<char> = plain.chars().collect();
    let mut out = Vec::with_capacity(punctuated.chars().count());
    let mut plain_index = 0usize;
    for ch in punctuated.chars() {
        // 标点与空白都可能是模型插入的，不算作原文的一部分
        if is_punct(ch) || ch.is_whitespace() {
            out.push(None);
            continue;
        }
        if plain_index < plain_chars.len() && plain_chars[plain_index] == ch {
            out.push(Some(plain_index));
            plain_index += 1;
        } else {
            // 文本不完全一致（极少）：退化成按下标比例对应
            let ratio_index = (out.len() * plain_chars.len())
                .checked_div(punctuated.chars().count().max(1))
                .unwrap_or(0)
                .min(plain_chars.len().saturating_sub(1));
            out.push(Some(ratio_index));
        }
    }
    out
}

/// 把一段识别结果切成句子。
///
/// - `plain`：识别原始文本（token 拼接的结果）
/// - `punctuated`：标点模型输出（通常是 plain 加上标点；没有标点模型时传 plain）
/// - `tokens` / `timestamps`：token 序列与每个 token 的起始时间（**秒**）
/// - `span_ms`：本段的绝对可用时间范围（时间戳缺失时按字数比例分配）
pub fn build_sentences(
    plain: &str,
    punctuated: &str,
    tokens: &[String],
    timestamps: &[f32],
    span_ms: (u32, u32),
) -> Vec<RawSentence> {
    let text = if punctuated.trim().is_empty() {
        plain
    } else {
        punctuated
    };
    if text.trim().is_empty() {
        return Vec::new();
    }
    let owners = char_token_map(plain, tokens);
    let aligned = align(plain, text);
    let pieces = split_pieces(text);
    let has_timestamps = !timestamps.is_empty() && tokens.len() == timestamps.len();

    // 时间戳缺失时按可见字数分配整段时长
    let visible_total: usize = text.chars().filter(|c| !is_punct(*c)).count().max(1);
    let span_len = span_ms.1.saturating_sub(span_ms.0).max(1);
    let mut consumed = 0usize;

    let mut sentences = Vec::new();
    for (piece, from, to) in pieces {
        let piece_visible = piece.chars().filter(|c| !is_punct(*c)).count();
        if piece_visible == 0 {
            continue;
        }
        // 片段覆盖的原始字符下标范围
        let plain_indexes: Vec<usize> = aligned[from..=to].iter().filter_map(|v| *v).collect();
        let (start_ms, end_ms) = if has_timestamps && !plain_indexes.is_empty() {
            let first_char = plain_indexes[0];
            let last_char = *plain_indexes.last().unwrap();
            let first_token = owners
                .get(first_char)
                .copied()
                .unwrap_or(0)
                .min(timestamps.len() - 1);
            let last_token = owners
                .get(last_char)
                .copied()
                .unwrap_or(first_token)
                .min(timestamps.len() - 1);
            let start = (timestamps[first_token].max(0.0) * 1000.0) as u32;
            // 末句没有「下一个 token」可参考，就在最后一个 token 的时间上留一点尾巴
            let last_start = (timestamps[last_token].max(0.0) * 1000.0) as u32;
            let end = if last_token + 1 < timestamps.len() {
                (timestamps[last_token + 1].max(0.0) * 1000.0) as u32
            } else {
                last_start + TAIL_MS
            };
            (start, end.max(start + MIN_SENTENCE_MS))
        } else {
            let start = span_ms.0 + (span_len as usize * consumed / visible_total) as u32;
            consumed += piece_visible;
            let end = span_ms.0 + (span_len as usize * consumed / visible_total) as u32;
            (start, end.max(start + MIN_SENTENCE_MS))
        };
        let text = piece.trim().to_string();
        if text.is_empty() {
            continue;
        }
        sentences.push(RawSentence {
            start_ms,
            end_ms,
            text,
        });
    }
    sentences
}

/// 句子的起止时间整体平移到段落内的绝对位置。
pub fn offset_sentences(sentences: &mut [RawSentence], offset_ms: u32) {
    for sentence in sentences.iter_mut() {
        sentence.start_ms += offset_ms;
        sentence.end_ms += offset_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_of(text: &str) -> Vec<String> {
        text.chars().map(|c| c.to_string()).collect()
    }

    #[test]
    fn 按标点断句并给出时间戳() {
        let plain = "今天讨论了三件事第一是进度第二是风险第三是人力";
        let punctuated = "今天讨论了三件事。第一是进度，第二是风险，第三是人力。";
        let tokens = tokens_of(plain);
        let timestamps: Vec<f32> = (0..tokens.len()).map(|i| 0.2 * i as f32).collect();
        let sentences = build_sentences(plain, punctuated, &tokens, &timestamps, (0, 6000));
        assert_eq!(
            sentences.len(),
            3,
            "应切成 3 句（弱标点不切碎）：{sentences:?}"
        );
        assert_eq!(sentences[0].text, "今天讨论了三件事。");
        assert_eq!(sentences[0].start_ms, 0);
        assert!(sentences[1].start_ms >= sentences[0].end_ms.saturating_sub(200));
        for pair in sentences.windows(2) {
            assert!(pair[0].start_ms <= pair[1].start_ms, "时间戳应单调");
        }
        assert!(sentences.last().unwrap().end_ms <= 6000);
    }

    #[test]
    fn 弱标点不切碎短句() {
        let plain = "好的一共三件事";
        let punctuated = "好的，一共三件事。";
        let tokens = tokens_of(plain);
        let timestamps: Vec<f32> = (0..tokens.len()).map(|i| 0.1 * i as f32).collect();
        let sentences = build_sentences(plain, punctuated, &tokens, &timestamps, (0, 3000));
        assert_eq!(
            sentences.len(),
            1,
            "「好的，」太短应并入下一句：{sentences:?}"
        );
        assert_eq!(sentences[0].text, "好的，一共三件事。");
    }

    #[test]
    fn 没有时间戳时按字数分配() {
        let plain = "第一句内容第二句内容";
        let punctuated = "第一句内容。第二句内容。";
        let sentences = build_sentences(plain, punctuated, &[], &[], (1000, 5000));
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].start_ms, 1000);
        assert!(sentences[0].end_ms <= sentences[1].start_ms);
        assert_eq!(sentences[1].end_ms, 5000);
    }

    #[test]
    fn 英文bpe续接标记不影响对齐() {
        let tokens = vec![
            "yes@@".to_string(),
            "terday".to_string(),
            "was".to_string(),
            "星".to_string(),
            "期".to_string(),
            "一".to_string(),
        ];
        let plain = "yesterdaywas星期一";
        let punctuated = "yesterday was 星期一。";
        let timestamps = vec![0.1, 0.4, 0.9, 1.2, 1.4, 1.6];
        let sentences = build_sentences(plain, punctuated, &tokens, &timestamps, (0, 3000));
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0].start_ms, 100);
        assert!(sentences[0].end_ms >= 1600);
    }

    #[test]
    fn 空文本不产生句子() {
        assert!(build_sentences("", "", &[], &[], (0, 1000)).is_empty());
        assert!(build_sentences("", "。", &[], &[], (0, 1000)).is_empty());
    }

    #[test]
    fn 句子整体平移() {
        let mut sentences = vec![RawSentence {
            start_ms: 100,
            end_ms: 200,
            text: "你好。".into(),
        }];
        offset_sentences(&mut sentences, 500);
        assert_eq!(sentences[0].start_ms, 600);
        assert_eq!(sentences[0].end_ms, 700);
    }
}
