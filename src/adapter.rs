//! The model behind the front.
//!
//! The front knows text, voices and audio formats. An adapter knows one model server:
//! how to ask whether it is up, which voices it has, and how to turn one piece of text
//! into samples. Adding a model means adding an adapter here and a name in
//! [`crate::config`]; nothing else changes.

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

/// A text-to-speech model.
pub trait Synth: Send + Sync + 'static {
    /// Model name, reported as `engine` by `/health`.
    fn engine(&self) -> &str;
    /// Where the model is, reported as `upstream` by `/health`.
    fn location(&self) -> &str;
    /// Sample rate of what [`Synth::synth`] returns.
    fn rate(&self) -> u32;
    /// The voices, in the order they are listed to callers.
    fn voices(&self) -> &[String];
    /// Is the model reachable? The error is shown to the caller of `/health`.
    fn check(&self) -> impl Future<Output = Result<(), String>> + Send;
    /// One piece of text in one voice, as mono 16-bit little-endian samples at
    /// [`Synth::rate`]. The error is shown to the caller with status 502.
    fn synth(&self, text: &str, voice: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
}

/// NVIDIA Magpie TTS served by NeMo-Speech.cpp (`nemo-speech serve`, `:8808` by default).
pub struct Magpie {
    url: String,
    model: String,
    rate: u32,
    voices: Vec<String>,
    http: reqwest::Client,
}

impl Magpie {
    /// The five voices this Magpie build has. The Python front listed them the same way.
    pub const VOICES: [&'static str; 5] = ["John", "Sofia", "Aria", "Jason", "Leo"];

    pub fn new(url: &str, model: &str, rate: u32) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            rate,
            voices: Self::VOICES.iter().map(|v| v.to_string()).collect(),
            // The model is on this machine or this network: never through a proxy.
            http: reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(5)).build().expect("HTTP client"),
        }
    }
}

impl Synth for Magpie {
    fn engine(&self) -> &str {
        &self.model
    }

    fn location(&self) -> &str {
        &self.url
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    fn voices(&self) -> &[String] {
        &self.voices
    }

    async fn check(&self) -> Result<(), String> {
        let reply = self
            .http
            .get(format!("{}/v1/models", self.url))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if reply.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP Error {}", reply.status()))
        }
    }

    async fn synth(&self, text: &str, voice: &str) -> Result<Vec<u8>, String> {
        let payload = json!({
            "model": self.model, "input": text, "voice": voice,
            "language": "en-US", "response_format": "pcm",
        });
        let reply = self
            .http
            .post(format!("{}/v1/audio/speech", self.url))
            .timeout(Duration::from_secs(120))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("{} unreachable: {e}", self.model))?;
        let status = reply.status();
        let body = reply.bytes().await.map_err(|e| format!("{} unreachable: {e}", self.model))?;
        if !status.is_success() {
            let shown = &body[..body.len().min(200)];
            return Err(format!("{} {}: {}", self.model, status.as_u16(), String::from_utf8_lossy(shown)));
        }
        Ok(body.to_vec())
    }
}

/// A stand-in model for tests and for trying the front without a GPU. Every character
/// of text becomes 100 samples of one fixed value, so the length of an answer is
/// predictable and nothing audible comes out.
pub struct Mock {
    pub rate: u32,
    pub voices: Vec<String>,
    /// When set, both `check` and `synth` fail with this message.
    pub fail: Option<String>,
    /// Every `(text, voice)` that was asked for, in order.
    pub calls: Mutex<Vec<(String, String)>>,
}

impl Mock {
    pub const SAMPLES_PER_CHAR: usize = 100;

    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            voices: Magpie::VOICES.iter().map(|v| v.to_string()).collect(),
            fail: None,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl Synth for Mock {
    fn engine(&self) -> &str {
        "mock"
    }

    fn location(&self) -> &str {
        "mock"
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    fn voices(&self) -> &[String] {
        &self.voices
    }

    async fn check(&self) -> Result<(), String> {
        self.fail.clone().map_or(Ok(()), Err)
    }

    async fn synth(&self, text: &str, voice: &str) -> Result<Vec<u8>, String> {
        self.calls.lock().unwrap().push((text.to_string(), voice.to_string()));
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        let samples = text.chars().count() * Self::SAMPLES_PER_CHAR;
        Ok([0x10u8, 0x00].repeat(samples))
    }
}
