pub mod bash;
pub mod bot;
pub mod commands;
pub mod config;
pub mod git;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod openrouter;
pub mod skills;
pub mod system_prompt;

pub use bot::GlowBot;
pub use config::Config;

/// Escape LLM output for Telegram MarkdownV2 using telegram-markdown-v2.
/// Parses the Markdown the LLM wrote and emits properly escaped V2.
pub fn escape_v2_safe(text: &str) -> String {
    match telegram_markdown_v2::convert_with_strategy(
        text,
        telegram_markdown_v2::UnsupportedTagsStrategy::Escape,
    ) {
        Ok(escaped) => escaped.trim_end().to_string(),
        Err(e) => {
            log::warn!("Markdown parse failed, using plain text: {}", e);
            text.to_string()
        }
    }
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
        assert_eq!(escape_v2_safe("**bold**"), "*bold*");
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
        // telegram-markdown-v2 converts - to Unicode bullet •
        let out = escape_v2_safe("- item");
        assert!(out.contains("item"), "got: {}", out);
    }

    #[test]
    fn test_preserve_numbered_list() {
        // telegram-markdown-v2 escapes 1. as 1\.
        let out = escape_v2_safe("1. item");
        assert!(out.contains("item"), "got: {}", out);
    }

    #[test]
    fn test_escape_tilde() {
        assert_eq!(escape_v2_safe("~5 min"), "\\~5 min");
    }

    #[test]
    fn test_preserve_heading_hash() {
        // telegram-markdown-v2 converts ## to *bold* (V2 has no headings)
        let out = escape_v2_safe("## Header");
        assert!(out.contains("Header"), "got: {}", out);
    }

    #[test]
    fn test_mixed_formatting() {
        let input = "I love **Rust** and `async`! (really)";
        let output = escape_v2_safe(input);
        assert!(output.contains("*Rust*"));
        assert!(output.contains("`async`"));
    }

    #[test]
    fn test_multiline_with_list() {
        let input = "Hello!\n- item one\n- item two\nThat is all.";
        let output = escape_v2_safe(input);
        assert!(output.contains("Hello\\!"));
        assert!(output.contains("That is all\\."));
    }
}
