//! The HTTP API. Routes, fields, status codes, messages and the spacing of the JSON
//! are those of the Python front (`docs/API.md`).

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};

use crate::adapter::Synth;
use crate::speakable::Speakable;
use crate::text::{chunk_text, clean_text, speech_control, MAX_INPUT};
use crate::{audio, pyjson};

/// Everything a request needs.
pub struct App<S> {
    pub synth: S,
    pub default_voice: String,
    pub reported_default_voice: String,
    pub ffmpeg: String,
    /// Writes numbers and the like as words before the text is cleaned. `None` = the
    /// text goes to the model as the Python front sent it.
    pub speakable: Option<Speakable>,
    /// Loudness of the answer (see [`audio::set_level`]); 0 = the model's own.
    pub level: f32,
}

pub fn router<S: Synth>(app: Arc<App<S>>) -> Router {
    Router::new()
        .route("/health", get(health::<S>))
        .route("/v1/models", get(models))
        .route("/v1/audio/voices", get(voices::<S>))
        .route("/voices", get(voices::<S>))
        .route("/v1/audio/speech", post(speech::<S>))
        // The Python front answered "not found" for every other path and for a known
        // path with the other method.
        .fallback(not_found)
        .method_not_allowed_fallback(not_found)
        .with_state(app)
}

fn reply(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .expect("valid response")
}

fn json_reply(status: StatusCode, value: Value) -> Response {
    reply(status, "application/json", pyjson::to_vec(&value))
}

fn error(status: StatusCode, message: &str) -> Response {
    let kind = if status.as_u16() < 500 { "invalid_request_error" } else { "server_error" };
    json_reply(status, json!({"error": {"message": message, "type": kind}}))
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not found")
}

async fn health<S: Synth>(State(app): State<Arc<App<S>>>) -> Response {
    match app.synth.check().await {
        Ok(()) => json_reply(
            StatusCode::OK,
            json!({
                "ok": true, "voices": app.synth.voices().len(), "default": app.reported_default_voice,
                "upstream": app.synth.location(), "engine": app.synth.engine(), "rate": app.synth.rate(),
            }),
        ),
        Err(e) => json_reply(StatusCode::SERVICE_UNAVAILABLE, json!({"ok": false, "error": e})),
    }
}

/// Kept exactly as the Python front answered: it still names the two earlier models.
/// Callers may match on these ids, so changing the list is a decision, not a cleanup.
async fn models() -> Response {
    json_reply(
        StatusCode::OK,
        json!({"object": "list", "data": [{"id": "voxcpm2", "object": "model"}, {"id": "qwen3-tts", "object": "model"}]}),
    )
}

async fn voices<S: Synth>(State(app): State<Arc<App<S>>>) -> Response {
    let list: Vec<Value> = app.synth.voices().iter().map(|v| json!({"id": v, "label": v})).collect();
    json_reply(StatusCode::OK, json!({"voices": list, "default": app.reported_default_voice}))
}

/// The model's name for the voice the caller asked for, in any letter case; the default
/// voice for anything else (OpenAI's voice names, `default`, nothing at all).
fn resolve_voice<'a>(voices: &'a [String], default: &'a str, asked: Option<&Value>) -> &'a str {
    let asked = asked.and_then(Value::as_str).unwrap_or("").to_lowercase();
    voices.iter().find(|v| v.to_lowercase() == asked).map_or(default, String::as_str)
}

async fn speech<S: Synth>(State(app): State<Arc<App<S>>>, body: Bytes) -> Response {
    let bad = |message: &str| error(StatusCode::BAD_REQUEST, message);

    let req: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v @ Value::Object(_)) => v,
            _ => return bad("body must be JSON"),
        }
    };

    let input = match req.get("input") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) if n.as_f64() != Some(0.0) => n.to_string(),
        _ => String::new(),
    };
    let normalize = match req.get("normalize") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(on)) => *on,
        Some(_) => return bad("normalize must be true or false"),
    };
    let input = match &app.speakable {
        Some(speakable) if normalize => speakable.apply(&input),
        _ => input,
    };
    let text = clean_text(&input);
    if text.is_empty() {
        return bad("input is empty");
    }
    if text.chars().count() > MAX_INPUT {
        return bad(&format!("input longer than {MAX_INPUT} chars"));
    }

    let instructions = match req.get("instructions") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => return bad("instructions must be a string"),
    };
    // The style cue is checked and cut off the text, then not used: Magpie has none.
    let (text, _cue) = match speech_control(&text, instructions) {
        Ok(parts) => parts,
        Err(e) => return bad(e),
    };

    let format = match req.get("response_format") {
        None | Some(Value::Null) => "mp3".to_string(),
        Some(Value::String(s)) if s.is_empty() => "mp3".to_string(),
        Some(Value::String(s)) => s.to_lowercase(),
        Some(other) => other.to_string(),
    };
    let Some(content_type) = audio::content_type(&format) else {
        return bad(&format!("unsupported response_format {format}"));
    };

    let voice = resolve_voice(app.synth.voices(), &app.default_voice, req.get("voice"));
    let rate = app.synth.rate();
    let mut pcm = Vec::new();
    for (i, piece) in chunk_text(&text).iter().enumerate() {
        let samples = match app.synth.synth(piece, voice).await {
            Ok(samples) => samples,
            Err(e) => return error(StatusCode::BAD_GATEWAY, &e),
        };
        if i > 0 {
            pcm.extend_from_slice(&audio::gap(rate));
        }
        pcm.extend_from_slice(&samples);
    }

    audio::set_level(&mut pcm, app.level);
    let out = match audio::encode(&app.ffmpeg, pcm, &format, rate).await {
        Ok(out) => out,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    eprintln!("speech voice={voice} fmt={format} chars={} bytes={}", text.chars().count(), out.len());
    reply(StatusCode::OK, content_type, out)
}
