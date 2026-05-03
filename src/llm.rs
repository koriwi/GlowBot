use crate::openrouter::{ChatCompletionRequest, ChatCompletionResponse, OpenRouterClient};
use async_trait::async_trait;

/// Trait abstracting the LLM backend, allowing mocking in tests.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse>;
}

/// Real OpenRouter implementation.
pub struct OpenRouterBackend {
    client: OpenRouterClient,
}

impl OpenRouterBackend {
    pub fn new(api_key: String) -> Self {
        Self {
            client: OpenRouterClient::new(api_key),
        }
    }
}

#[async_trait]
impl LlmBackend for OpenRouterBackend {
    async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse> {
        self.client.chat_completion(request).await
    }
}

/// A mock LLM backend for testing.
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// A programmable mock LLM backend.
    pub struct MockLlmBackend {
        pub responses: Mutex<Vec<ChatCompletionResponse>>,
        pub should_error: Mutex<bool>,
    }

    impl MockLlmBackend {
        pub fn new() -> Self {
            Self {
                responses: Mutex::new(Vec::new()),
                should_error: Mutex::new(false),
            }
        }

        pub fn add_response(&self, response: ChatCompletionResponse) {
            self.responses.lock().unwrap().push(response);
        }

        pub fn set_error(&self, error: bool) {
            *self.should_error.lock().unwrap() = error;
        }
    }

    impl Default for MockLlmBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl LlmBackend for MockLlmBackend {
        async fn chat_completion(
            &self,
            _request: &ChatCompletionRequest,
        ) -> anyhow::Result<ChatCompletionResponse> {
            if *self.should_error.lock().unwrap() {
                return Err(anyhow::anyhow!("Mock LLM error"));
            }
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                // Return a simple text response
                Ok(ChatCompletionResponse {
                    choices: vec![crate::openrouter::Choice {
                        message: crate::openrouter::AssistantMessage {
                            content: Some("Mock response".into()),
                            tool_calls: None,
                            role: Some("assistant".into()),
                        },
                        finish_reason: Some("stop".into()),
                    }],
                })
            } else {
                Ok(responses.remove(0))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::openrouter::ToolCall;

        #[tokio::test]
        async fn test_mock_empty_returns_default() {
            let mock = MockLlmBackend::new();
            let req = ChatCompletionRequest {
                model: "test".into(),
                messages: vec![],
                tools: None,
                tool_choice: None,
            };
            let resp = mock.chat_completion(&req).await.unwrap();
            assert_eq!(
                resp.choices[0].message.content.as_deref(),
                Some("Mock response")
            );
        }

        #[tokio::test]
        async fn test_mock_returns_queued_responses() {
            let mock = MockLlmBackend::new();
            mock.add_response(ChatCompletionResponse {
                choices: vec![crate::openrouter::Choice {
                    message: crate::openrouter::AssistantMessage {
                        content: Some("First".into()),
                        tool_calls: None,
                        role: Some("assistant".into()),
                    },
                    finish_reason: Some("stop".into()),
                }],
            });
            mock.add_response(ChatCompletionResponse {
                choices: vec![crate::openrouter::Choice {
                    message: crate::openrouter::AssistantMessage {
                        content: Some("Second".into()),
                        tool_calls: Some(vec![ToolCall {
                            id: "call_1".into(),
                            call_type: "function".into(),
                            function: crate::openrouter::FunctionCall {
                                name: "bash".into(),
                                arguments: "{}".into(),
                            },
                        }]),
                        role: Some("assistant".into()),
                    },
                    finish_reason: Some("tool_calls".into()),
                }],
            });

            let req = ChatCompletionRequest {
                model: "test".into(),
                messages: vec![],
                tools: None,
                tool_choice: None,
            };
            let resp1 = mock.chat_completion(&req).await.unwrap();
            assert_eq!(resp1.choices[0].message.content.as_deref(), Some("First"));

            let resp2 = mock.chat_completion(&req).await.unwrap();
            assert_eq!(
                resp2.choices[0].message.tool_calls.as_ref().unwrap()[0].id,
                "call_1"
            );

            // Third call gets default since queue is empty
            let resp3 = mock.chat_completion(&req).await.unwrap();
            assert_eq!(
                resp3.choices[0].message.content.as_deref(),
                Some("Mock response")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openrouter_backend_new() {
        let backend = OpenRouterBackend::new("test-key".into());
        // Just verify it constructs successfully
        let _ = backend;
    }
}
