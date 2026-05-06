use teloxide::prelude::*;
use teloxide::types::{ChatId, ParseMode};

/// Maximum number of characters per Telegram message chunk.
/// 4096 is the Telegram hard limit; we use 4000 to leave breathing room
/// for MarkdownV2 escaping overhead (special chars like `.`, `!`, `-` get
/// backslash-escaped, which can inflate the byte count).
const MAX_CHUNK_CHARS: usize = 4000;

/// Send a text message to a Telegram chat, automatically splitting
/// into multiple messages if the text exceeds the character limit.
///
/// Each chunk is sent with MarkdownV2 parsing. If parsing fails
/// (e.g. unbalanced formatting), the chunk is retried as plain text.
/// Multi-part messages get a "*Part X/Y*" header.
pub async fn send_message(bot: &teloxide::Bot, chat_id: ChatId, text: &str) {
    let chunks = split_for_telegram(text, MAX_CHUNK_CHARS);
    let total = chunks.len();
    for (i, chunk) in chunks.iter().enumerate() {
        let header = if total > 1 {
            format!("*Part {}/{}*\n\n", i + 1, total)
        } else {
            String::new()
        };
        let msg = format!("{}{}", header, chunk);
        let escaped = crate::escape_v2_safe(&msg);
        let result = bot
            .send_message(chat_id, &escaped)
            .parse_mode(ParseMode::MarkdownV2)
            .await;
        if let Err(e) = result {
            log::warn!("MarkdownV2 parse failed, sending as plain text: {}", e);
            let _ = bot.send_message(chat_id, &msg).await;
        }
    }
}

/// Split text into chunks of at most `max_chars` characters, preferring
/// newline boundaries so we don't break mid-word or mid-sentence.
///
/// Works on Unicode character count (not byte count), so multi-byte
/// characters like `ü`, `é`, or emoji are counted as single characters.
fn split_for_telegram(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let char_count = remaining.chars().count();
        if char_count <= max_chars {
            chunks.push(remaining.to_string());
            break;
        }
        // Find the byte offset of the character at position max_chars.
        // char_indices() yields (byte_offset, char), so .nth(max_chars)
        // gives the byte offset where the (max_chars+1)-th char starts.
        let byte_at = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(remaining.len());
        // Prefer splitting at a newline before this position (if any)
        let split_at = if let Some(pos) = remaining[..byte_at].rfind('\n') {
            pos + 1 // include the newline in the current chunk
        } else {
            byte_at
        };
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_text() {
        let result = split_for_telegram("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_split_exact_fit() {
        let result = split_for_telegram("1234567890", 10);
        assert_eq!(result, vec!["1234567890"]);
    }

    #[test]
    fn test_split_at_newline() {
        let text = "first line\nsecond line\nthird line";
        // "first line\n" = 11 chars, "second line\n" = 12 chars, "third line" = 10 chars
        // max_len=20: first chunk is "first line\n" (11), then "second line\n" (12),
        // then "third line" (10) — three chunks total.
        let result = split_for_telegram(text, 20);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "first line\n");
        assert_eq!(result[1], "second line\n");
        assert_eq!(result[2], "third line");
    }

    #[test]
    fn test_split_no_newline() {
        let text = "abcdefghijklmnop"; // 16 chars
        let result = split_for_telegram(text, 5);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], "abcde");
        assert_eq!(result[1], "fghij");
        assert_eq!(result[2], "klmno");
        assert_eq!(result[3], "p");
    }

    #[test]
    fn test_split_empty() {
        let result = split_for_telegram("", 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_multiple_newlines() {
        let text = "a\n\nb\n\nc"; // 7 chars
        let result = split_for_telegram(text, 3);
        // rfind('\n') in first 3 chars finds pos 3 → "a\n\n"
        // then "b\n\n", then "c" fits.
        assert_eq!(result, vec!["a\n\n", "b\n\n", "c"]);
    }

    #[test]
    fn test_split_multibyte_utf8() {
        // ü is 2 bytes but 1 char. 4 lines of 3–4 chars each = 15 chars total.
        // max_chars=6 → splits as: "abü\n" (4), "cdü\n" (4), "efü\n" (4), "ghü" (3).
        let text = "abü\ncdü\nefü\nghü";
        let result = split_for_telegram(text, 6);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], "abü\n");
        assert_eq!(result[1], "cdü\n");
        assert_eq!(result[2], "efü\n");
        assert_eq!(result[3], "ghü");
    }

    #[test]
    fn test_split_emoji() {
        // 😀 is 4 bytes but 1 char
        let text = "😀😀😀😀😀"; // 5 chars
        let result = split_for_telegram(text, 3);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "😀😀😀");
        assert_eq!(result[1], "😀😀");
    }
}
