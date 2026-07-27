//! Best-effort smoke tests against real OpenAI-compatible services.
//!
//! Every test here is `#[ignore]`d so a normal `cargo test` never touches
//! the network. Run explicitly with:
//!
//!     cargo test --test live_backend_smoke -- --ignored --nocapture
//!
//! Requires two services reachable on this machine:
//!   - `http://localhost:1234`  (LM Studio-like; model `qwen/qwen3.5-9b`,
//!     lenient about model names, so it cannot be used to provoke a
//!     misconfigured/4xx response)
//!   - `http://localhost:11434` (Ollama's OpenAI-compatible endpoint;
//!     model `qwen3.5:cloud`; returns 404 for an unknown model name)
//!
//! Both can take anywhere from 10s to several minutes per completion
//! because the loaded model streams a reasoning trace even for trivial
//! prompts, hence the generous per-request timeout below.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::Duration;

use trai::translator::{FallbackTranslator, OpenAITranslator, Translator};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(240);
// Both servers expose their OpenAI-compatible surface under /v1; without
// that prefix they still answer with a 200/404 but on the wrong routes
// (LM Studio: a JSON "unexpected endpoint" error even on 200; Ollama: a
// generic 404 page, not the real "model not found" body).
const LM_STUDIO_URL: &str = "http://localhost:1234/v1";
const LM_STUDIO_MODEL: &str = "qwen/qwen3.5-9b";
const OLLAMA_URL: &str = "http://localhost:11434/v1";

fn unreachable_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

fn lm_studio_backend() -> std::sync::Arc<dyn Translator> {
    std::sync::Arc::new(OpenAITranslator::new(
        LM_STUDIO_URL,
        LM_STUDIO_MODEL,
        "es",
        None,
        REQUEST_TIMEOUT,
    ))
}

// Accepts `responses.len()` sequential connections on `addr` and replies
// to each with the corresponding canned HTTP response, in order. Mirrors
// the raw-TcpListener pattern in `src/translator/openai.rs`'s test module,
// extended to serve more than one response since this fake stands in for
// a backend that first answers a probe() and then a translate() call.
fn serve_sequential_responses(
    addr: SocketAddr,
    responses: Vec<(&'static str, String)>,
) -> thread::JoinHandle<()> {
    let listener = TcpListener::bind(addr).unwrap();
    thread::spawn(move || {
        for (status_line, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_buffer = [0u8; 1024];
            let _ = stream.read(&mut request_buffer).unwrap();

            let response = format!(
                "{status_line}\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\
\r\n\
{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    })
}

#[test]
#[ignore]
fn lan_unreachable_every_line_served_by_local_and_marked_degraded() {
    let unreachable: std::sync::Arc<dyn Translator> = std::sync::Arc::new(OpenAITranslator::new(
        unreachable_addr(),
        "irrelevant-model",
        "es",
        None,
        REQUEST_TIMEOUT,
    ));
    let fallback = FallbackTranslator::new(
        vec![unreachable, lm_studio_backend()],
        Duration::from_secs(60),
    );

    let first = fallback
        .translate("hello, how are you today?", &[])
        .expect("reachable fallback backend should answer");
    assert!(first.degraded);
    assert!(!first.text.is_empty());

    let second = fallback
        .translate("what is the weather like where you are?", &[])
        .expect("reachable fallback backend should answer");
    assert!(second.degraded);
    assert!(!second.text.is_empty());
}

#[test]
#[ignore]
fn lan_recovers_mid_recording_and_new_rows_stop_being_marked_within_one_reprobe_interval() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let primary_addr = listener.local_addr().unwrap();
    drop(listener);

    let primary: std::sync::Arc<dyn Translator> = std::sync::Arc::new(OpenAITranslator::new(
        format!("http://{primary_addr}"),
        "irrelevant-model",
        "es",
        None,
        REQUEST_TIMEOUT,
    ));
    let reprobe_interval = Duration::from_secs(2);
    let fallback = FallbackTranslator::new(vec![primary, lm_studio_backend()], reprobe_interval);

    let first = fallback
        .translate("hello, how are you today?", &[])
        .expect("secondary backend should answer while primary is down");
    assert!(
        first.degraded,
        "must fail over while primary is unreachable"
    );

    // Bring the primary "back" by serving a fake OpenAI-compatible server
    // on its exact port: a 200 for the probe() call the reprobe issues,
    // then a valid chat-completion body for the translate() call that
    // follows promotion.
    let server = serve_sequential_responses(
        primary_addr,
        vec![
            ("HTTP/1.1 200 OK", r#"{"data":[]}"#.to_string()),
            (
                "HTTP/1.1 200 OK",
                r#"{"choices":[{"message":{"content":"hola de nuevo"}}]}"#.to_string(),
            ),
        ],
    );

    thread::sleep(reprobe_interval + Duration::from_millis(500));

    let second = fallback
        .translate("welcome back", &[])
        .expect("promoted primary backend should answer");
    assert!(
        !second.degraded,
        "primary must be promoted back within one reprobe interval"
    );
    assert_eq!(second.text, "hola de nuevo");

    server.join().unwrap();
}

#[test]
#[ignore]
fn misconfigured_model_name_surfaces_error_without_demoting_backend() {
    let bad_model_backend: std::sync::Arc<dyn Translator> =
        std::sync::Arc::new(OpenAITranslator::new(
            OLLAMA_URL,
            "definitely-not-a-real-model",
            "es",
            None,
            REQUEST_TIMEOUT,
        ));
    let fallback = FallbackTranslator::new(
        vec![bad_model_backend, lm_studio_backend()],
        Duration::from_secs(60),
    );

    let first_err = fallback
        .translate("hello", &[])
        .expect_err("unknown model name should be rejected as misconfigured");
    assert!(first_err.is_misconfigured());
    assert!(
        first_err.to_string().len() > 10,
        "error message should carry real detail, got: {first_err}"
    );

    let second_err = fallback
        .translate("hello again", &[])
        .expect_err("misconfigured backend must not be demoted by failover");
    assert!(
        second_err.is_misconfigured(),
        "a demoted backend would very likely have succeeded via the real fallback instead"
    );
}
