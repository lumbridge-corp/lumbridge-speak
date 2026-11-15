//! The HTTP API against the mock model, and the Magpie adapter against a stand-in
//! Magpie. Bodies marked "observed" are real answers of the Python front
//! `:8803`, 2026-11-09; this front must answer byte for byte the same.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use lumbridge_speak::adapter::{Magpie, Mock, Synth};
use lumbridge_speak::audio;
use lumbridge_speak::config::Config;
use lumbridge_speak::server::{router, App};
use serde_json::{json, Value};
use tower::ServiceExt;

const SPEECH: &str = "/v1/audio/speech";

fn app<S: Synth>(synth: S) -> Arc<App<S>> {
    Arc::new(App { synth, default_voice: "Leo".into(), reported_default_voice: "default".into(), ffmpeg: "ffmpeg".into(), speakable: None, level: 0.0 })
}

/// The mock model at the given rate.
fn mock(rate: u32) -> Arc<App<Mock>> {
    app(Mock::new(rate))
}

struct Answer {
    status: StatusCode,
    content_type: String,
    body: Vec<u8>,
}

impl Answer {
    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).expect("text body")
    }
}

async fn send<S: Synth>(app: &Arc<App<S>>, method: &str, path: &str, body: &str) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router(app.clone()).oneshot(request).await.unwrap();
    Answer {
        status: response.status(),
        content_type: response.headers().get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default(),
        body: response.into_body().collect().await.unwrap().to_bytes().to_vec(),
    }
}

async fn speak<S: Synth>(app: &Arc<App<S>>, body: Value) -> Answer {
    send(app, "POST", SPEECH, &body.to_string()).await
}

// ── GET /health ──────────────────────────────────────────────────────────────

/// A stand-in Magpie: answers `/v1/models`, and `/v1/audio/speech` with `reply`,
/// remembering what it was asked.
async fn fake_magpie(reply: (StatusCode, Vec<u8>)) -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let service = Router::new().route("/v1/models", get(|| async { Json(json!({"data": [], "object": "list"})) })).route(
        SPEECH,
        post(move |Json(asked): Json<Value>| {
            log.lock().unwrap().push(asked);
            let reply = reply.clone();
            async move { reply }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });
    (url, seen)
}

#[tokio::test]
async fn health_has_the_shape_and_spacing_of_the_python_front() {
    let (url, _) = fake_magpie((StatusCode::OK, vec![])).await;
    let a = send(&app(Magpie::new(&url, "magpietts", 22_050)), "GET", "/health", "").await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.content_type, "application/json");
    // Observed, with the stand-in's address in place of http://127.0.0.1:8808.
    let observed = r#"{"ok": true, "voices": 5, "default": "default", "upstream": "http://127.0.0.1:8808", "engine": "magpietts", "rate": 22050}"#;
    assert_eq!(a.text(), observed.replace("http://127.0.0.1:8808", &url));
    // What the model manager's health check looks for.
    assert!(a.text().contains("\"ok\": true"));
}

#[tokio::test]
async fn health_is_503_when_the_model_is_down() {
    let mut down = Mock::new(22_050);
    down.fail = Some("connection refused".into());
    let a = send(&app(down), "GET", "/health", "").await;
    assert_eq!(a.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(a.text(), r#"{"ok": false, "error": "connection refused"}"#);

    // The real adapter, pointed at a port nothing listens on.
    let a = send(&app(Magpie::new("http://127.0.0.1:1", "magpietts", 22_050)), "GET", "/health", "").await;
    assert_eq!(a.status, StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_slice(&a.body).unwrap();
    assert_eq!(body["ok"], json!(false));
    assert!(body["error"].is_string());
}

// ── GET /v1/models, /v1/audio/voices, /voices ────────────────────────────────

#[tokio::test]
async fn models_is_the_fixed_list_of_the_python_front() {
    let a = send(&mock(22_050), "GET", "/v1/models", "").await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.content_type, "application/json");
    // Observed.
    assert_eq!(a.text(), r#"{"object": "list", "data": [{"id": "voxcpm2", "object": "model"}, {"id": "qwen3-tts", "object": "model"}]}"#);
}

#[tokio::test]
async fn voices_on_both_paths() {
    // Observed.
    let observed = r#"{"voices": [{"id": "John", "label": "John"}, {"id": "Sofia", "label": "Sofia"}, {"id": "Aria", "label": "Aria"}, {"id": "Jason", "label": "Jason"}, {"id": "Leo", "label": "Leo"}], "default": "default"}"#;
    for path in ["/v1/audio/voices", "/voices"] {
        let a = send(&mock(22_050), "GET", path, "").await;
        assert_eq!(a.status, StatusCode::OK, "{path}");
        assert_eq!(a.content_type, "application/json");
        assert_eq!(a.text(), observed, "{path}");
    }
}

// ── POST /v1/audio/speech ────────────────────────────────────────────────────

#[tokio::test]
async fn speech_wav_is_a_wav_at_the_models_rate() {
    let app = mock(22_050);
    let a = speak(&app, json!({"input": "Testing one two three.", "voice": "Leo", "response_format": "wav"})).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.content_type, "audio/wav"); // observed
    let samples = "Testing one two three.".len() * Mock::SAMPLES_PER_CHAR * 2;
    assert_eq!(a.body.len(), 44 + samples);
    // The header layout of the observed answer: RIFF, PCM, mono, 22050 Hz, 16-bit.
    assert_eq!(&a.body[..4], b"RIFF");
    assert_eq!(a.body[4..8], (36 + samples as u32).to_le_bytes());
    assert_eq!(&a.body[8..16], b"WAVEfmt ");
    assert_eq!(a.body[16..36], [16, 0, 0, 0, 1, 0, 1, 0, 0x22, 0x56, 0, 0, 0x44, 0xac, 0, 0, 2, 0, 16, 0]);
    assert_eq!(&a.body[36..40], b"data");
    assert_eq!(a.body[40..44], (samples as u32).to_le_bytes());
    assert_eq!(*app.synth.calls.lock().unwrap(), [("Testing one two three.".to_string(), "Leo".to_string())]);
}

#[tokio::test]
async fn speech_pcm_is_raw_samples() {
    // At 24000 Hz nothing has to be resampled, so this needs no ffmpeg.
    let a = speak(&mock(24_000), json!({"input": "Hello.", "response_format": "PCM"})).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.content_type, "audio/pcm"); // observed
    assert_eq!(a.body, [0x10u8, 0x00].repeat(6 * Mock::SAMPLES_PER_CHAR));
}

#[tokio::test]
async fn voice_is_matched_in_any_case_and_falls_back_to_the_default() {
    let app = mock(24_000);
    for voice in [json!("aria"), json!("JASON"), json!("alloy"), json!("default"), json!(""), Value::Null, json!(7)] {
        let a = speak(&app, json!({"input": "Hi.", "voice": voice, "response_format": "pcm"})).await;
        assert_eq!(a.status, StatusCode::OK);
    }
    assert_eq!(speak(&app, json!({"input": "Hi.", "response_format": "pcm"})).await.status, StatusCode::OK);
    let used: Vec<String> = app.synth.calls.lock().unwrap().iter().map(|(_, voice)| voice.clone()).collect();
    assert_eq!(used, ["Aria", "Jason", "Leo", "Leo", "Leo", "Leo", "Leo", "Leo"]);
}

#[tokio::test]
async fn long_text_is_spoken_in_pieces_joined_by_silence() {
    let app = mock(24_000);
    let first = format!("{}.", "a".repeat(120));
    let second = format!("{}.", "b".repeat(120));
    let a = speak(&app, json!({"input": format!("{first} {second}"), "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK);
    let calls = app.synth.calls.lock().unwrap().clone();
    assert_eq!(calls, [(first.clone(), "Leo".to_string()), (second.clone(), "Leo".to_string())]);
    let piece = 121 * Mock::SAMPLES_PER_CHAR * 2;
    let gap = audio::gap(24_000).len();
    assert_eq!(gap, 3600 * 2);
    assert_eq!(a.body.len(), piece + gap + piece);
    assert!(a.body[piece..piece + gap].iter().all(|&b| b == 0), "the gap is silence");
}

#[tokio::test]
async fn text_is_cleaned_and_the_style_cue_is_cut_off() {
    let app = mock(24_000);
    let a = speak(&app, json!({"input": "(softly) Read the [docs](https://x.test) **now**.", "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(app.synth.calls.lock().unwrap()[0].0, "Read the docs now .");
}

#[tokio::test]
async fn speech_errors_are_those_of_the_python_front() {
    let app = mock(24_000);
    // Every body here was observed.
    let cases = [
        (r#"{"input":""}"#, "input is empty"),
        ("notjson", "body must be JSON"),
        (r#"{"input":"hi","response_format":"xyz"}"#, "unsupported response_format xyz"),
        (r#"{"input":"hi","instructions":5}"#, "instructions must be a string"),
        (r#"{"input":"hi","instructions":"(a)"}"#, "instructions must be at most 300 chars without parentheses"),
    ];
    for (body, message) in cases {
        let a = send(&app, "POST", SPEECH, body).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(a.content_type, "application/json");
        assert_eq!(a.text(), format!(r#"{{"error": {{"message": "{message}", "type": "invalid_request_error"}}}}"#), "{body}");
    }
    // Read from the code, not observed.
    let a = speak(&app, json!({"input": "x".repeat(4097)})).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert_eq!(a.text(), r#"{"error": {"message": "input longer than 4096 chars", "type": "invalid_request_error"}}"#);
    assert_eq!(speak(&app, json!({"input": "x".repeat(4096), "response_format": "pcm"})).await.status, StatusCode::OK);
    // An empty body is an empty request, as in the Python front.
    assert!(send(&app, "POST", SPEECH, "").await.text().contains("input is empty"));
    // The first failing check answers: empty input before a bad format.
    assert!(send(&app, "POST", SPEECH, r#"{"response_format":"xyz"}"#).await.text().contains("input is empty"));
    let spoken: String = app.synth.calls.lock().unwrap().iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(spoken, "x".repeat(4096), "only the one good request reached the model");
}

#[tokio::test]
async fn a_failing_model_is_a_502() {
    let mut broken = Mock::new(24_000);
    broken.fail = Some("magpietts unreachable: connection refused".into());
    let a = speak(&app(broken), json!({"input": "Hi."})).await;
    assert_eq!(a.status, StatusCode::BAD_GATEWAY);
    assert_eq!(a.text(), r#"{"error": {"message": "magpietts unreachable: connection refused", "type": "server_error"}}"#);
}

#[tokio::test]
async fn everything_else_is_not_found() {
    let app = mock(24_000);
    // Observed: GET /nope and POST /nope.
    let observed = r#"{"error": {"message": "not found", "type": "invalid_request_error"}}"#;
    for (method, path) in [("GET", "/nope"), ("POST", "/nope"), ("GET", SPEECH), ("POST", "/health"), ("GET", "/")] {
        let a = send(&app, method, path, "").await;
        assert_eq!(a.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(a.text(), observed, "{method} {path}");
    }
}

// ── The Magpie adapter ───────────────────────────────────────────────────────

#[tokio::test]
async fn magpie_is_asked_the_way_the_python_front_asked() {
    let samples = vec![1u8, 0, 2, 0, 3, 0];
    let (url, seen) = fake_magpie((StatusCode::OK, samples.clone())).await;
    let app = app(Magpie::new(&url, "magpietts", 24_000));
    let a = speak(&app, json!({"input": "Hello there.", "voice": "sofia", "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.body, samples);
    assert_eq!(
        *seen.lock().unwrap(),
        [json!({"model": "magpietts", "input": "Hello there.", "voice": "Sofia", "language": "en-US", "response_format": "pcm"})]
    );
}

#[tokio::test]
async fn magpie_errors_become_502_with_its_answer() {
    // Magpie's real answer to a voice it does not have (observed through a speech front).
    let refusal = r#"{"error":{"code":null,"message":"unknown voice_name 'Nobody'","param":null,"type":"invalid_request_error"}}"#;
    let (url, _) = fake_magpie((StatusCode::BAD_REQUEST, refusal.as_bytes().to_vec())).await;
    let a = speak(&app(Magpie::new(&url, "magpietts", 24_000)), json!({"input": "Hi."})).await;
    assert_eq!(a.status, StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_slice(&a.body).unwrap();
    assert_eq!(body["error"]["type"], "server_error");
    assert_eq!(body["error"]["message"], format!("magpietts 400: {refusal}"));

    let a = speak(&app(Magpie::new("http://127.0.0.1:1", "magpietts", 24_000)), json!({"input": "Hi."})).await;
    assert_eq!(a.status, StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_slice(&a.body).unwrap();
    assert!(body["error"]["message"].as_str().unwrap().starts_with("magpietts unreachable: "));
}

// ── Formats that need ffmpeg ─────────────────────────────────────────────────
// Run with `cargo test -- --include-ignored` on a machine that has ffmpeg.

#[tokio::test]
#[ignore = "needs ffmpeg"]
async fn pcm_is_resampled_to_24000_hz() {
    let a = speak(&mock(22_050), json!({"input": "Testing one two three.", "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK, "{}", String::from_utf8_lossy(&a.body));
    assert_eq!(a.content_type, "audio/pcm");
    // 2200 samples at 22050 Hz are 2394.6 at 24000 Hz. (The observed answer had the
    // same ratio: 69,632 bytes at 22050 Hz came back as 75,790.)
    let samples = a.body.len() / 2;
    assert!((2390..=2400).contains(&samples), "{samples} samples");
}

#[tokio::test]
#[ignore = "needs ffmpeg"]
async fn compressed_formats_come_out_of_ffmpeg() {
    let app = mock(22_050);
    let text = "A sentence long enough to make more than one frame of audio in every format.";
    for (format, content_type, magic) in [
        ("mp3", "audio/mpeg", &b"\xff"[..]),
        ("opus", "audio/ogg", &b"OggS"[..]),
        ("aac", "audio/aac", &b"\xff\xf1"[..]),
        ("flac", "audio/flac", &b"fLaC"[..]),
    ] {
        let a = speak(&app, json!({"input": text, "response_format": format})).await;
        assert_eq!(a.status, StatusCode::OK, "{format}: {}", String::from_utf8_lossy(&a.body));
        assert_eq!(a.content_type, content_type, "{format}");
        let id3 = format == "mp3" && a.body.starts_with(b"ID3");
        assert!(id3 || a.body.starts_with(magic), "{format} starts with {:02x?}", &a.body[..4]);
    }
    // No format given means mp3.
    let a = speak(&app, json!({"input": text})).await;
    assert_eq!(a.content_type, "audio/mpeg");
}

#[tokio::test]
async fn a_missing_ffmpeg_is_a_500() {
    let app = Arc::new(App {
        synth: Mock::new(22_050),
        default_voice: "Leo".into(),
        reported_default_voice: "default".into(),
        ffmpeg: "/nonexistent/ffmpeg".into(),
        speakable: None,
        level: 0.0,
    });
    let a = speak(&app, json!({"input": "Hi."})).await;
    assert_eq!(a.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = serde_json::from_slice(&a.body).unwrap();
    assert_eq!(body["error"]["type"], "server_error");
    assert!(body["error"]["message"].as_str().unwrap().starts_with("ffmpeg: "));
}

// ── Numbers as words, loudness ───────────────────────────────────────────────

fn speaking(level: f32) -> Arc<App<Mock>> {
    let say = [("NixOS".to_string(), "Nix O S".to_string())];
    let speakable = lumbridge_speak::speakable::Speakable::new(say.iter().map(|(a, b)| (a, b)), &[]).unwrap();
    Arc::new(App {
        synth: Mock::new(24_000),
        default_voice: "Leo".into(),
        reported_default_voice: "default".into(),
        ffmpeg: "ffmpeg".into(),
        speakable: Some(speakable),
        level,
    })
}

#[tokio::test]
async fn written_forms_reach_the_model_as_words() {
    let app = speaking(0.0);
    let a = speak(&app, json!({"input": "PR #73 for NixOS merged at 4:05 PM and cost $0.19.", "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK);
    let a = speak(&app, json!({"input": "See [the docs](https://x.test/a) in `~/src/app`, 95% done.", "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK);
    let calls: Vec<String> = app.synth.calls.lock().unwrap().iter().map(|(text, _)| text.clone()).collect();
    assert_eq!(
        calls,
        ["PR number seventy-three for Nix O S merged at four oh five P M and cost nineteen cents.", "See the docs in app , ninety-five percent done."]
    );
}

#[tokio::test]
async fn a_request_can_ask_for_its_text_as_written() {
    let app = speaking(0.0);
    let a = speak(&app, json!({"input": "PR 73.", "normalize": false, "response_format": "pcm"})).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(app.synth.calls.lock().unwrap()[0].0, "PR 73.");
    let a = speak(&app, json!({"input": "PR 73.", "normalize": "no"})).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert_eq!(a.text(), r#"{"error": {"message": "normalize must be true or false", "type": "invalid_request_error"}}"#);
}

#[tokio::test]
async fn the_answer_is_brought_to_the_level_asked_for() {
    // The mock's samples are all 0x0010 = 16: below the floor, so they stay.
    let quiet = speak(&speaking(0.9), json!({"input": "Hi.", "response_format": "pcm"})).await;
    assert_eq!(quiet.body, [0x10u8, 0x00].repeat(3 * Mock::SAMPLES_PER_CHAR));
}

// ── Settings ─────────────────────────────────────────────────────────────────

#[test]
fn the_example_file_shows_the_defaults() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lumbridge-speak.example.toml");
    assert_eq!(Config::load(Some(&path), |_| None).unwrap(), Config::default());
}

// ── Against a real Magpie ────────────────────────────────────────────────────
// Off unless LUMBRIDGE_SPEAK_TEST_MAGPIE names a server that speaks Magpie's contract.
// It asks for one short sentence. Nothing is started and nothing listens: the request
// goes through the router in this process.

#[tokio::test]
#[ignore = "needs a real Magpie: set LUMBRIDGE_SPEAK_TEST_MAGPIE"]
async fn a_real_magpie_accepts_what_the_adapter_sends() {
    let Ok(url) = std::env::var("LUMBRIDGE_SPEAK_TEST_MAGPIE") else {
        eprintln!("skipped: LUMBRIDGE_SPEAK_TEST_MAGPIE is not set");
        return;
    };
    let app = app(Magpie::new(&url, "magpietts", 22_050));
    let health = send(&app, "GET", "/health", "").await;
    assert_eq!(health.status, StatusCode::OK, "{}", health.text());

    let a = speak(&app, json!({"input": "Testing one two three.", "voice": "leo", "response_format": "wav"})).await;
    assert_eq!(a.status, StatusCode::OK, "{}", String::from_utf8_lossy(&a.body));
    assert_eq!(a.content_type, "audio/wav");
    let seconds = (a.body.len() - 44) as f64 / 2.0 / 22_050.0;
    eprintln!("real Magpie: {} bytes, {seconds:.2} s of audio", a.body.len());
    // The Python front answered the same sentence with 1.58 s; the model varies a little.
    assert!((0.8..3.0).contains(&seconds), "{seconds} s");
    assert!(a.body[44..].chunks(2).any(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs() > 1000), "not silence");
}
