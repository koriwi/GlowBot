use super::{ChatMessage, ToolDefinition};

/// Rough token estimation for a string of text.
/// Uses ~1 token per 4 characters (common English approximation).
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).saturating_add(3) / 4
}

/// Estimate tokens for a `ChatMessage`.
/// Counts role overhead (~4 tokens) plus content text tokens and reasoning if present.
pub fn estimate_message_tokens(msg: &ChatMessage) -> u64 {
    let text = msg.text_content();
    // Role overhead + content; tool_calls JSON adds overhead too
    let mut total = 4 + estimate_tokens(&text);
    if let Some(tcs) = &msg.tool_calls {
        let json = serde_json::to_string(tcs).unwrap_or_default();
        total += estimate_tokens(&json);
    }
    if msg.tool_call_id.is_some() {
        total += 4; // small overhead for tool result messages
    }
    if let Some(ref reasoning) = msg.reasoning {
        total += estimate_tokens(reasoning);
    }
    total
}

/// Estimate tokens for a slice of messages.
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Estimate tokens for tool definitions by serializing them.
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u64 {
    let json = serde_json::to_string(tools).unwrap_or_default();
    estimate_tokens(&json)
}

/// Safety margin multiplier for token estimates.
/// Since `estimate_tokens` is a rough approximation, we multiply our
/// estimates by this factor so we stay well under the real limit.
pub const TOKEN_ESTIMATE_MARGIN: f64 = 0.75;

/// Reserve tokens for the model's response.
pub const RESPONSE_RESERVE_TOKENS: u64 = 8192;

/// Build a message list that fits within the model's context length.
///
/// - `context_limit`: the model's max context length from OpenRouter (0 = unknown)
/// - `head`: messages that are always preserved (e.g. system prompt, task header)
/// - `history`: prior conversation messages that may be trimmed from oldest
/// - `turn`: current turn messages that are always preserved
/// - `tools`: active tool definitions
///
/// Returns `(messages, was_trimmed)` where `messages` is the list to send.
/// If the context limit is unknown, nothing is trimmed.
pub fn build_trimmed_request(
    context_limit: u64,
    head: &[ChatMessage],
    history: &[ChatMessage],
    turn: &[ChatMessage],
    tools: &[ToolDefinition],
) -> (Vec<ChatMessage>, bool) {
    if context_limit == 0 {
        let mut msgs = head.to_vec();
        msgs.extend(history.iter().cloned());
        msgs.extend(turn.iter().cloned());
        return (msgs, false);
    }

    let effective_limit = (context_limit as f64 * TOKEN_ESTIMATE_MARGIN) as u64;
    let head_tokens = estimate_messages_tokens(head);
    let tools_tokens = estimate_tools_tokens(tools);
    let turn_tokens = estimate_messages_tokens(turn);

    let fixed_cost = head_tokens
        .saturating_add(tools_tokens)
        .saturating_add(turn_tokens)
        .saturating_add(RESPONSE_RESERVE_TOKENS);

    if fixed_cost >= effective_limit {
        log::warn!(
            "Context limit too small: fixed cost {} >= effective limit {} (context limit {})",
            fixed_cost,
            effective_limit,
            context_limit
        );
        // Still try to send head + turn only, history is impossible
        let mut msgs = head.to_vec();
        msgs.extend(turn.iter().cloned());
        return (msgs, true);
    }

    let mut history_budget = effective_limit.saturating_sub(fixed_cost);
    let mut trimmed_history: Vec<ChatMessage> = Vec::new();
    let mut trimmed = false;

    // Walk history oldest → newest, keeping messages while they fit.
    for msg in history {
        let cost = estimate_message_tokens(msg);
        if cost <= history_budget {
            trimmed_history.push(msg.clone());
            history_budget = history_budget.saturating_sub(cost);
        } else {
            trimmed = true;
        }
    }

    // Strip orphaned tool results: when we couldn't fit an
    // assistant_tool_calls message but a cheaper tool_result did fit,
    // the result starts with orphaned tool messages. Remove them.
    trimmed_history = strip_orphaned_tool_results(&trimmed_history);

    if trimmed {
        let dropped = history.len().saturating_sub(trimmed_history.len());
        log::info!(
            "Trimmed {} old messages to fit context limit {} (effective {})",
            dropped,
            context_limit,
            effective_limit
        );
    }

    let mut msgs = head.to_vec();
    msgs.extend(trimmed_history);
    msgs.extend(turn.iter().cloned());
    (msgs, trimmed)
}

/// Trim a flat message list by dropping messages from the *middle*, preserving
/// `preserve_prefix` head messages and `preserve_suffix` tail messages.
/// Used for heartbeat tasks where `messages` is a single flat list.
pub fn trim_message_list(
    messages: &[ChatMessage],
    preserve_prefix: usize,
    preserve_suffix: usize,
) -> Vec<ChatMessage> {
    if messages.len() <= preserve_prefix + preserve_suffix {
        return messages.to_vec();
    }
    let mut result = Vec::with_capacity(preserve_prefix + preserve_suffix + 1);
    result.extend_from_slice(&messages[..preserve_prefix]);
    // Insert a placeholder summary message
    let dropped = messages.len() - preserve_prefix - preserve_suffix;
    result.push(ChatMessage::system(&format!(
        "... {} earlier messages omitted to fit context limit ...",
        dropped
    )));
    // Strip orphaned tool_results from the tail: if the assistant_tool_calls
    // was in the dropped middle, the tail may start with orphaned tool messages.
    let suffix = &messages[messages.len() - preserve_suffix..];
    let cleaned_suffix = strip_orphaned_tool_results(suffix);
    result.extend(cleaned_suffix);
    result
}

/// Strip orphaned tool result messages from a message list.
///
/// An orphaned tool result is a message with role "tool" whose `tool_call_id`
/// doesn't match any preceding assistant message's `tool_calls`. This can
/// happen when:
/// - `load_messages` slides its window, dropping an assistant_tool_calls
///   message but keeping its subsequent tool_results
/// - `build_trimmed_request` skips an expensive assistant_tool_calls
///   message but fits a cheaper subsequent tool_result
/// - `trim_message_list` drops the middle containing assistant_tool_calls
///   but preserves the suffix with orphaned tool_results
///
/// DeepSeek and other strict APIs reject requests with orphaned tool messages.
pub fn strip_orphaned_tool_results(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Track which tool_call_ids have been "opened" by preceding
    // assistant_tool_calls messages.
    let mut open_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stripped = 0usize;

    let result: Vec<ChatMessage> = messages
        .iter()
        .filter(|msg| {
            // Update open_ids when we see an assistant with tool_calls
            if msg.role == "assistant" {
                if let Some(tcs) = &msg.tool_calls {
                    // Each new assistant_tool_calls resets the set (a new batch of results)
                    open_ids.clear();
                    for tc in tcs {
                        open_ids.insert(tc.id.clone());
                    }
                }
                return true;
            }

            // For tool messages, check if the tool_call_id is expected
            if msg.role == "tool" {
                if let Some(ref id) = msg.tool_call_id {
                    if open_ids.contains(id) {
                        return true;
                    }
                }
                // Orphaned tool result — drop it
                stripped += 1;
                return false;
            }

            // Non-tool, non-assistant messages (user, system) — always keep
            true
        })
        .cloned()
        .collect();

    if stripped > 0 {
        log::info!(
            "Stripped {} orphaned tool result(s) from message list",
            stripped
        );
    }

    result
}

/// Format token usage as a human-readable string like "37k/252k (15%)".
/// When the context limit is unknown, reports the used tokens if available,
/// or "no data yet" if nothing has been recorded.
pub fn format_context_usage(used: u64, limit: u64) -> String {
    if limit == 0 {
        if used > 0 {
            let used_k = used / 1000;
            format!("{}k used (context limit unknown)", used_k)
        } else {
            "no token data yet".to_string()
        }
    } else {
        let pct = ((used as f64 / limit as f64) * 100.0).round() as u64;
        let used_k = used / 1000;
        let limit_k = limit / 1000;
        format!("{}k/{}k ({}%)", used_k, limit_k, pct)
    }
}
