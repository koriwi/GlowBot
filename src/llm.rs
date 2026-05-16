use crate::openrouter::{ChatCompletionRequest, ChatCompletionResponse, OpenRouterClient};
use async_trait::async_trait;

/// Trait abstracting the LLM backend, allowing mocking in tests.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> anyhow::Result<ChatCompletionResponse>;

    /// Generate embeddings for a text string.
    async fn embeddings(&self, model: &str, input: &str) -> anyhow::Result<Vec<f32>>;
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

    async fn embeddings(&self, model: &str, input: &str) -> anyhow::Result<Vec<f32>> {
        self.client.embeddings(model, input).await
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
        pub embedding_responses: Mutex<Vec<Vec<f32>>>,
        pub should_error: Mutex<bool>,
    }

    impl MockLlmBackend {
        pub fn new() -> Self {
            Self {
                responses: Mutex::new(Vec::new()),
                embedding_responses: Mutex::new(Vec::new()),
                should_error: Mutex::new(false),
            }
        }

        fn lock_responses(&self) -> std::sync::MutexGuard<'_, Vec<ChatCompletionResponse>> {
            self.responses.lock().unwrap_or_else(|e| e.into_inner())
        }

        fn lock_embeddings(&self) -> std::sync::MutexGuard<'_, Vec<Vec<f32>>> {
            self.embedding_responses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
        }

        fn check_error(&self) -> bool {
            *self.should_error.lock().unwrap_or_else(|e| e.into_inner())
        }

        pub fn add_response(&self, response: ChatCompletionResponse) {
            self.lock_responses().push(response);
        }

        pub fn add_embedding(&self, embedding: Vec<f32>) {
            self.lock_embeddings().push(embedding);
        }

        pub fn set_error(&self, error: bool) {
            *self.should_error.lock().unwrap_or_else(|e| e.into_inner()) = error;
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
            if self.check_error() {
                return Err(anyhow::anyhow!("Mock LLM error"));
            }
            let mut responses = self.lock_responses();
            if responses.is_empty() {
                // Return a simple text response
                Ok(ChatCompletionResponse {
                    choices: vec![crate::openrouter::Choice {
                        message: crate::openrouter::AssistantMessage {
                            content: Some("Mock response".into()),
                            tool_calls: None,
                            role: Some("assistant".into()),
                            reasoning: None,
                            ..Default::default()
                        },
                        finish_reason: Some("stop".into()),
                    }],
                    ..Default::default()
                })
            } else {
                Ok(responses.remove(0))
            }
        }

        async fn embeddings(&self, _model: &str, _input: &str) -> anyhow::Result<Vec<f32>> {
            if self.check_error() {
                return Err(anyhow::anyhow!("Mock embedding error"));
            }
            let mut embeddings = self.lock_embeddings();
            if embeddings.is_empty() {
                Ok(vec![0.1, 0.2, 0.3, 0.4])
            } else {
                Ok(embeddings.remove(0))
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
                modalities: None,
                image_config: None,
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
                        reasoning: None,
                        ..Default::default()
                    },
                    finish_reason: Some("stop".into()),
                }],
                ..Default::default()
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
                        reasoning: None,
                        ..Default::default()
                    },
                    finish_reason: Some("tool_calls".into()),
                }],
                ..Default::default()
            });

            let req = ChatCompletionRequest {
                model: "test".into(),
                messages: vec![],
                tools: None,
                tool_choice: None,
                modalities: None,
                image_config: None,
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

        #[tokio::test]
        async fn test_mock_embeddings_default() {
            let mock = MockLlmBackend::new();
            let result = mock.embeddings("any-model", "test").await.unwrap();
            assert_eq!(result.len(), 4);
            assert!((result[0] - 0.1).abs() < 1e-6);
        }

        #[tokio::test]
        async fn test_mock_embeddings_queued() {
            let mock = MockLlmBackend::new();
            mock.add_embedding(vec![4.0, 5.0, 6.0]);
            let result = mock.embeddings("any-model", "test").await.unwrap();
            assert_eq!(result, vec![4.0, 5.0, 6.0]);
            // Second call returns default
            let result2 = mock.embeddings("any-model", "test").await.unwrap();
            assert_eq!(result2, vec![0.1, 0.2, 0.3, 0.4]);
        }

        #[tokio::test]
        async fn test_mock_embeddings_error() {
            let mock = MockLlmBackend::new();
            mock.set_error(true);
            let result = mock.embeddings("any-model", "test").await;
            assert!(result.is_err());
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
