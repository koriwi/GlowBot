pub mod bash;
pub mod bot;
pub mod commands;
pub mod config;
pub mod git;
pub mod llm;
pub mod memory;
pub mod openrouter;
pub mod skills;
pub mod system_prompt;

pub use bot::GlowBot;
pub use config::Config;

/// Escape MarkdownV2 reserved characters that LLMs commonly output in natural
/// text, while preserving intentional formatting.
pub fn escape_v2_safe(text: &str) -> String {
    let always_escape = ['!', '(', ')', '+', '=', '|', '{', '}', '#', '~', '>'];
    let mut result = String::with_capacity(text.len() + 32);

    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if always_escape.contains(&ch) {
                result.push('\\');
            }
            if ch == '.' || ch == '-' {
                let is_list_marker = (i == 0 && i + 1 < chars.len() && chars[i + 1] == ' ')
                    || (ch == '.'
                        && i == 1
                        && chars[0].is_ascii_digit()
                        && i + 1 < chars.len()
                        && chars[i + 1] == ' ');
                if !is_list_marker {
                    result.push('\\');
                }
            }
            result.push(ch);
        }
        result.push('\n');
    }
    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_exclamation() {
        assert_eq!(escape_v2_safe("Hello!"), "Hello\\!");
    }

    #[test]
    fn test_escape_parentheses() {
        assert_eq!(escape_v2_safe("Hi (there)"), "Hi \\(there\\)");
    }

    #[test]
    fn test_preserve_bold() {
        assert_eq!(escape_v2_safe("**bold**"), "**bold**");
    }

    #[test]
    fn test_preserve_code() {
        assert_eq!(escape_v2_safe("`code`"), "`code`");
    }

    #[test]
    fn test_escape_dot_in_text() {
        assert_eq!(escape_v2_safe("end."), "end\\.");
    }

    #[test]
    fn test_escape_dash_in_compound_word() {
        assert_eq!(escape_v2_safe("well-known"), "well\\-known");
    }

    #[test]
    fn test_preserve_list_dash() {
        assert_eq!(escape_v2_safe("- item"), "- item");
    }

    #[test]
    fn test_preserve_numbered_list() {
        assert_eq!(escape_v2_safe("1. item"), "1. item");
    }

    #[test]
    fn test_escape_tilde() {
        assert_eq!(escape_v2_safe("~5 min"), "\\~5 min");
    }

    #[test]
    fn test_escape_hash() {
        assert_eq!(escape_v2_safe("topic #1"), "topic \\#1");
    }

    #[test]
    fn test_mixed_formatting() {
        let input = "I love **Rust** and `async`! (really)";
        let output = escape_v2_safe(input);
        assert!(output.contains("**Rust**"));
        assert!(output.contains("`async`"));
        assert!(output.contains("\\!"));
        assert!(output.contains("\\("));
        assert!(output.contains("\\)"));
    }

    #[test]
    fn test_multiline_with_list() {
        let input = "Hello!\n- item one\n- item two\nThat is all.";
        let output = escape_v2_safe(input);
        assert!(output.contains("Hello\\!"));
        assert!(output.contains("- item one"));
        assert!(output.contains("- item two"));
        assert!(output.contains("That is all\\."));
    }
}
