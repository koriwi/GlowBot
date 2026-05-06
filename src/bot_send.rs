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

/// Split text into chunks of at most `max_len` characters, preferring
/// newline boundaries so we don't break mid-word or mid-sentence.
fn split_for_telegram(text: &str, max_len: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        // Prefer splitting at a newline within the chunk window
        let split_at = if let Some(pos) = remaining[..max_len].rfind('\n') {
            pos + 1 // include the newline in the current chunk
        } else {
            max_len
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
        // rfind('\n') in "a\n\n"[..3] finds pos 2, splits at 3 → "a\n\n"
        // then "b\n\n" (rfind finds pos 2 again) → "b\n\n"
        // then "c" fits.
        assert_eq!(result, vec!["a\n\n", "b\n\n", "c"]);
    }
}
