use std::time::Duration;

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{TranslateError, Translator};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OpenAITranslator {
    base_url: String,
    model: String,
    target_language: String,
    api_key: Option<String>,
    client: Client,
}

impl OpenAITranslator {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        target_language: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url,
            model: model.into(),
            target_language: target_language.into(),
            api_key,
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("failed to build reqwest blocking client for OpenAI"),
        }
    }

    pub(crate) fn build_request_payload(
        model: &str,
        target_language: &str,
        context: &[String],
        text: &str,
    ) -> Value {
        let context_block = if context.is_empty() {
            "No prior context available.".to_string()
        } else {
            context
                .iter()
                .enumerate()
                .map(|(idx, line)| format!("{idx}: {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": format!("Translate the next utterance into {target_language}.")
                },
                {
                    "role": "user",
                    "content": format!(
                        "Context:\n{context_block}\n\nText:\n{text}"
                    )
                }
            ]
        })
    }
}

impl Translator for OpenAITranslator {
    fn translate(&self, text: &str, context: &[String]) -> Result<String, TranslateError> {
        let body = Self::build_request_payload(&self.model, &self.target_language, context, text);

        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self.client.post(&url).json(&body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request
            .send()
            .map_err(|e| TranslateError::new(format!("translate request failed: {e}")))?;

        let response = response.error_for_status().map_err(|e| {
            let status = e
                .status()
                .map_or_else(|| "unknown status".to_string(), |status| status.to_string());
            TranslateError::new(format!("translate request failed with status: {status}"))
        })?;

        let raw_body = response
            .text()
            .map_err(|e| TranslateError::new(format!("failed to read translate response: {e}")))?;

        let response: ChatCompletionResponse = serde_json::from_str(&raw_body)
            .map_err(|e| TranslateError::new(format!("failed to parse translate response: {e}")))?;

        response
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .ok_or_else(|| TranslateError::new("translate response missing content"))
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_translate_response(
        status_line: &str,
        response_body: &str,
    ) -> (OpenAITranslator, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = response_body.to_string();
        let status_line = status_line.to_string();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_buffer = [0u8; 1024];
            let _ = stream.read(&mut request_buffer).unwrap();

            let response = format!(
                "{status_line}\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\
\r\n\
{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let translator =
            OpenAITranslator::new(format!("http://{address}"), "gpt-4o-mini", "es", None);
        (translator, server)
    }

    #[test]
    fn new_strips_trailing_slashes_from_base_url() {
        let translator = OpenAITranslator::new("http://example.com///", "gpt-4o-mini", "es", None);

        assert_eq!(translator.base_url, "http://example.com");
    }

    #[test]
    fn request_body_includes_target_language_context_and_text() {
        let context = vec!["t1".to_string(), "t2".to_string()];
        let payload =
            OpenAITranslator::build_request_payload("gpt-4o-mini", "es", &context, "text");

        assert_eq!(payload["model"], json!("gpt-4o-mini"));

        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.contains("es"));

        let user = messages[1]["content"].as_str().unwrap();
        assert!(user.contains("Context:"));
        assert!(user.contains("t1"));
        assert!(user.contains("t2"));
        assert!(user.contains("Text:"));
        assert!(user.contains("text"));
    }

    #[test]
    fn translate_non_2xx_response_returns_error() {
        let (translator, server) =
            serve_translate_response("HTTP/1.1 500 Internal Server Error", r#"internal error"#);
        let err = translator
            .translate("hello", &[])
            .expect_err("non-2xx response should fail");

        assert!(err
            .to_string()
            .contains("translate request failed with status"));
        assert!(err.to_string().contains("500"));

        server.join().unwrap();
    }

    #[test]
    fn translate_returns_content_from_choices() {
        let (translator, server) = serve_translate_response(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"hola"}}]}"#,
        );
        let translation = translator
            .translate("hello", &[])
            .expect("translation should succeed");

        assert_eq!(translation, "hola");

        server.join().unwrap();
    }

    #[test]
    fn translate_missing_content_returns_error() {
        let (translator, server) = serve_translate_response("HTTP/1.1 200 OK", r#"{"choices":[]}"#);
        let err = translator
            .translate("hello", &[])
            .expect_err("empty choices should fail");

        assert_eq!(err.to_string(), "translate response missing content");

        server.join().unwrap();
    }
}
