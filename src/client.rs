//! The `speak` command: ask a front for audio and play it on this machine, hand a line
//! to another machine that speaks for us, list and audition the voices, and check that
//! what the voices say can be understood.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::process::Command;

use crate::config::Config;
use crate::speakable::Speakable;
use crate::text::{chunk_text, clean_text};

/// What `speak` was asked to do with a text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Say {
    pub text: String,
    /// Voice name; `None` = the settings, then the front's default.
    pub voice: Option<String>,
    /// Playback speed; `None` = the settings.
    pub rate: Option<f32>,
    /// Speak on another machine (a name from `[client.targets]`) instead of this one.
    pub on: Option<String>,
    /// Write a WAV file instead of playing.
    pub out: Option<PathBuf>,
    /// Who is speaking, shown by a target that shows it.
    pub from: Option<String>,
}

fn http() -> reqwest::Client {
    // Fronts and targets are on this machine or this network: never through a proxy.
    reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(5)).build().expect("HTTP client")
}

/// The settings' own names, for writing text as words on this side.
pub fn speakable(config: &Config) -> Result<Speakable, String> {
    Speakable::new(config.say.iter(), &config.drop)
}

fn voice<'a>(config: &'a Config, asked: Option<&'a str>) -> Option<&'a str> {
    asked.or(Some(config.client.voice.as_str())).filter(|v| !v.is_empty())
}

/// One request to the front: WAV bytes for `text`.
async fn fetch(http: &reqwest::Client, front: &str, text: &str, voice: Option<&str>) -> Result<Vec<u8>, String> {
    let mut body = json!({"input": text, "response_format": "wav"});
    if let Some(v) = voice {
        body["voice"] = json!(v);
    }
    let reply = http
        .post(format!("{front}/v1/audio/speech"))
        .timeout(Duration::from_secs(180))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("the front at {front} did not answer ({e}). Is lumbridge-speak running?"))?;
    let status = reply.status();
    let bytes = reply.bytes().await.map_err(|e| format!("the front at {front}: {e}"))?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).into_owned());
        return Err(format!("the front answered {}: {message}", status.as_u16()));
    }
    Ok(bytes.to_vec())
}

// ── playing ──────────────────────────────────────────────────────────────────

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The program that plays a WAV file here. `LUMBRIDGE_SPEAK_PLAYER` names another one;
/// it is run with the file as its last argument.
fn player(file: &Path, rate: f32) -> Result<Command, String> {
    let mut command = if let Some(own) = std::env::var("LUMBRIDGE_SPEAK_PLAYER").ok().filter(|v| !v.is_empty()) {
        let mut parts = own.split_whitespace();
        let mut c = Command::new(parts.next().unwrap_or_default());
        c.args(parts);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("afplay");
        if (rate - 1.0).abs() > 0.01 {
            c.arg("-r").arg(format!("{rate}"));
        }
        c
    } else if let Some(found) = ["pw-play", "paplay", "aplay"].into_iter().find(|p| on_path(p)) {
        let mut c = Command::new(found);
        if found == "aplay" {
            c.arg("-q");
        }
        c
    } else {
        return Err("nothing here can play audio (looked for pw-play, paplay, aplay). Use --out FILE, or --on another machine.".into());
    };
    command.arg(file).stdin(Stdio::null()).stdout(Stdio::null()).kill_on_drop(true);
    Ok(command)
}

/// One `speak` plays at a time on a machine; the others wait their turn. The file holds
/// the process that has the turn and the player it started, so `speak stop` can end both.
struct Turn {
    file: PathBuf,
}

fn turn_file() -> PathBuf {
    std::env::temp_dir().join("lumbridge-speak.turn")
}

fn alive(pid: &str) -> bool {
    !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && std::process::Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

impl Turn {
    async fn take() -> Turn {
        let file = turn_file();
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&file) {
                Ok(_) => {
                    let turn = Turn { file };
                    turn.note(None);
                    return turn;
                }
                Err(_) => {
                    let holder = std::fs::read_to_string(&file).unwrap_or_default();
                    if !alive(holder.split_whitespace().next().unwrap_or("")) {
                        let _ = std::fs::remove_file(&file);
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
        }
    }

    fn note(&self, player: Option<u32>) {
        let line = match player {
            Some(p) => format!("{} {p}", std::process::id()),
            None => std::process::id().to_string(),
        };
        let _ = std::fs::write(&self.file, line);
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// Ends whatever `speak` is playing on this machine. Returns whether anything was.
pub fn stop() -> bool {
    let file = turn_file();
    let Ok(holder) = std::fs::read_to_string(&file) else { return false };
    let pids: Vec<&str> = holder.split_whitespace().filter(|p| alive(p)).collect();
    if pids.is_empty() {
        let _ = std::fs::remove_file(&file);
        return false;
    }
    let _ = std::process::Command::new("kill").args(&pids).status();
    let _ = std::fs::remove_file(&file);
    true
}

/// Plays the pieces in order, asking the front for the next piece while one plays, so
/// speech starts after the first sentence and not after the whole text.
async fn play(config: &Config, pieces: Vec<String>, voice: Option<&str>, rate: f32) -> Result<(), String> {
    let http = http();
    let front = config.client.front.clone();
    let voice = voice.map(str::to_string);
    let ask = |piece: String| {
        let (http, front, voice) = (http.clone(), front.clone(), voice.clone());
        tokio::spawn(async move { fetch(&http, &front, &piece, voice.as_deref()).await })
    };
    let mut pieces = pieces.into_iter();
    let Some(first) = pieces.next() else { return Err("nothing to say".into()) };
    // The first piece is fetched before waiting for the turn: a failure shows at once.
    let mut audio = ask(first).await.map_err(|e| e.to_string())??;
    let turn = Turn::take().await;
    let mut n = 0;
    loop {
        let next = pieces.next().map(&ask);
        let file = std::env::temp_dir().join(format!("lumbridge-speak-{}-{n}.wav", std::process::id()));
        n += 1;
        std::fs::write(&file, &audio).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut child = player(&file, rate)?.spawn().map_err(|e| format!("cannot start the audio player: {e}"))?;
        turn.note(child.id());
        let played = tokio::select! {
            status = child.wait() => status.map_err(|e| e.to_string()).map(|s| s.success()),
            _ = tokio::signal::ctrl_c() => {
                let _ = child.kill().await;
                let _ = std::fs::remove_file(&file);
                return Ok(());
            }
        };
        let _ = std::fs::remove_file(&file);
        if !played? {
            return Err("the audio player stopped with an error".into());
        }
        match next {
            Some(task) => audio = task.await.map_err(|e| e.to_string())??,
            None => return Ok(()),
        }
    }
}

/// `speak TEXT`.
pub async fn say(config: &Config, say: &Say) -> Result<(), String> {
    if say.text.trim().is_empty() {
        return Err("nothing to say".into());
    }
    if let Some(name) = &say.on {
        return hand_over(config, name, &say.text, say.from.as_deref()).await;
    }
    let voice = voice(config, say.voice.as_deref());
    if let Some(out) = &say.out {
        // Written as words here too: the front asked may be one that does not do it.
        let audio = fetch(&http(), &config.client.front, &speakable(config)?.apply(&say.text), voice).await?;
        return std::fs::write(out, audio).map_err(|e| format!("{}: {e}", out.display()));
    }
    // Written as words here, with this machine's names, then cut at sentence ends. The
    // front does the same again with its own names; words pass through it unchanged.
    let spoken = clean_text(&speakable(config)?.apply(&say.text));
    play(config, chunk_text(&spoken), voice, say.rate.unwrap_or(config.client.rate)).await
}

/// `speak --on NAME TEXT`: the other machine queues the line and speaks it.
async fn hand_over(config: &Config, name: &str, text: &str, from: Option<&str>) -> Result<(), String> {
    let Some(target) = config.client.targets.get(name) else {
        let known: Vec<&str> = config.client.targets.keys().map(String::as_str).collect();
        return Err(match known.is_empty() {
            true => format!("no machine named {name}: the settings have no [client.targets]"),
            false => format!("no machine named {name}. Known: {}", known.join(", ")),
        });
    };
    let mut request = http()
        .post(format!("{}/say", target.url.trim_end_matches('/')))
        .timeout(Duration::from_secs(8))
        .json(&json!({"text": text.split_whitespace().collect::<Vec<_>>().join(" "), "from": from.unwrap_or("speak")}));
    if !target.token_file.is_empty() {
        let path = match target.token_file.strip_prefix("~/") {
            Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
            None => PathBuf::from(&target.token_file),
        };
        let token = std::fs::read_to_string(&path).map_err(|e| format!("token for {name}, {}: {e}", path.display()))?;
        request = request.bearer_auth(token.trim());
    }
    let reply = request.send().await.map_err(|e| format!("{name} did not take it ({e}); it may be asleep"))?;
    if !reply.status().is_success() {
        return Err(format!("{name} answered {}", reply.status().as_u16()));
    }
    Ok(())
}

/// The front's voices and its default, as the front reports them.
pub async fn voices(config: &Config) -> Result<(Vec<String>, String), String> {
    let front = &config.client.front;
    let reply = http()
        .get(format!("{front}/v1/audio/voices"))
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| format!("the front at {front} did not answer ({e}). Is lumbridge-speak running?"))?;
    let listed: Value = reply.json().await.map_err(|e| format!("the front at {front}: {e}"))?;
    let names = listed["voices"].as_array().into_iter().flatten().filter_map(|v| v["id"].as_str().map(str::to_string)).collect();
    Ok((names, listed["default"].as_str().unwrap_or("").to_string()))
}

/// `speak demo`: every voice says who it is and one sentence, one after another.
pub async fn demo(config: &Config, sentence: &str, rate: Option<f32>) -> Result<(), String> {
    let (names, _) = voices(config).await?;
    for name in names {
        println!("{name}");
        let line = format!("This is {name}. {sentence}");
        play(config, vec![line], Some(&name), rate.unwrap_or(config.client.rate)).await?;
    }
    Ok(())
}

// ── checking that the voices can be understood ───────────────────────────────

/// Sentences that cover plain prose and every kind of writing the rules handle.
pub const CHECK_SENTENCES: [&str; 13] = [
    "Good morning. Three things finished overnight, and one of them needs you.",
    "I merged the pull request and deployed it to the office, so there is nothing you need to do right now.",
    "The quick brown fox jumps over the lazy dog, and then it sits quietly by the river until sunset.",
    "PR 73 merged at 4:05 PM and cost $0.19.",
    "The model runs at 48 tok/s with 128K context and uses 32GB of memory.",
    "We deployed v0.3.1 on 2026-11-10, and 95% of the tests pass (4307/4307).",
    "Dr. Smith paid $1,250.50 for 3 GPUs, e.g. two for training.",
    "Call 315-555-0137 before 9am tomorrow.",
    "The RTX 5090 is about 2x faster than the 4080, and it took 645 ms.",
    "The site is live at https://example.com/docs and the server listens on port 8041.",
    "Revenue grew from $1.5M to $20K per customer, i.e. 3rd place in 2026.",
    "It took 5-10 minutes, w/o any errors, 24/7.",
    // Repeated digits: every one must be heard (a tripled digit once lost one in the transcript).
    "The support line is 800-555-0199, and the backup is 315-555-0137.",
];

const SMALL: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
    "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 8] = ["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

fn small(word: &str) -> Option<u64> {
    if word == "oh" || word == "o" {
        return Some(0);
    }
    SMALL.iter().position(|w| *w == word).map(|i| i as u64)
}

fn tens(word: &str) -> Option<u64> {
    TENS.iter().position(|w| *w == word).map(|i| (i as u64 + 2) * 10)
}

fn scale(word: &str) -> Option<u64> {
    Some(match word {
        "thousand" => 1_000,
        "million" => 1_000_000,
        "billion" => 1_000_000_000,
        _ => return None,
    })
}

fn numeric(word: &str) -> bool {
    small(word).is_some() || tens(word).is_some() || word == "hundred" || word.bytes().all(|b| b.is_ascii_digit())
}

/// One number read from the start of `run`: its digits, and how many words it took.
/// "four thousand three hundred seven" is one number; "fifty ninety" is two.
fn number(run: &[&str]) -> (String, usize) {
    if run[0].bytes().all(|b| b.is_ascii_digit()) {
        return (run[0].to_string(), 1);
    }
    if matches!(run[0], "oh" | "o" | "zero") {
        return ("0".to_string(), 1);
    }
    let (mut total, mut current, mut used) = (0u64, 0u64, 0);
    let (mut had_tens, mut had_ones) = (false, false);
    while let Some(&word) = run.get(used) {
        if let Some(v) = tens(word).filter(|_| !had_tens && !had_ones) {
            current += v;
            had_tens = true;
        } else if let Some(v) = small(word).filter(|&v| !had_ones && v > 0 && (!had_tens || v < 10)) {
            current += v;
            had_ones = true;
        } else if word == "hundred" && (1..100).contains(&current) {
            current *= 100;
            (had_tens, had_ones) = (false, false);
        } else if let Some(s) = scale(word).filter(|_| current > 0) {
            total += current * s;
            current = 0;
            (had_tens, had_ones) = (false, false);
        } else {
            break;
        }
        used += 1;
    }
    ((total + current).to_string(), used.max(1))
}

/// The words of a text for comparing: lower case, numbers as one run of digits however
/// they were written or said, and letters said one by one joined ("p m" = "pm").
fn words(text: &str) -> Vec<String> {
    let spoken = crate::speakable::speakable(text).to_lowercase();
    let split: Vec<&str> = spoken
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty())
        .collect();
    // "a m" and "a.m." are "am".
    let mut raw: Vec<&str> = Vec::with_capacity(split.len());
    let mut k = 0;
    while k < split.len() {
        match (split[k], split.get(k + 1)) {
            ("a", Some(&"m")) => (raw.push("am"), k += 2).0,
            ("p", Some(&"m")) => (raw.push("pm"), k += 2).0,
            (word, _) => (raw.push(word), k += 1).0,
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut spelled = false; // the last word is letters said one by one
    let mut counted = false; // the last word is a number
    while i < raw.len() {
        let word = raw[i];
        let heard_as = word;
        if numeric(heard_as) || (scale(heard_as).is_some() && counted) {
            let mut run = vec![heard_as];
            run.extend(raw[i + 1..].iter().take_while(|w| numeric(w) || scale(w).is_some()));
            let (digits, used) = number(&run);
            match out.last_mut() {
                Some(last) if counted => last.push_str(&digits),
                _ => out.push(digits),
            }
            i += used;
            (counted, spelled) = (true, false);
            continue;
        }
        let letter = word.chars().count() == 1 && !matches!(word, "a" | "i");
        match out.last_mut() {
            Some(last) if letter && spelled => last.push_str(word),
            _ => out.push(word.to_string()),
        }
        (counted, spelled) = (false, letter);
        i += 1;
    }
    out
}

/// Share of the words of `said` that `heard` got wrong (word error rate). Both sides are
/// written as words first, so "4:05 PM" and "four oh five P M" count as the same.
pub fn word_errors(said: &str, heard: &str) -> f64 {
    let (r, h) = (words(said), words(heard));
    let mut row: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut prev = row.clone();
        std::mem::swap(&mut prev, &mut row);
        row[0] = i;
        for j in 1..=h.len() {
            row[j] = (prev[j] + 1).min(row[j - 1] + 1).min(prev[j - 1] + usize::from(r[i - 1] != h[j - 1]));
        }
    }
    row[h.len()] as f64 / r.len().max(1) as f64
}

async fn transcribe(http: &reqwest::Client, asr: &str, wav: &[u8]) -> Result<String, String> {
    let edge = format!("lumbridge-speak-{}", std::process::id());
    let mut body = format!(
        "--{edge}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{edge}--\r\n").as_bytes());
    let reply = http
        .post(format!("{asr}/v1/audio/transcriptions"))
        .timeout(Duration::from_secs(120))
        .header("Content-Type", format!("multipart/form-data; boundary={edge}"))
        .body(body)
        .send()
        .await
        .map_err(|e| format!("speech to text at {asr} did not answer ({e})"))?;
    let status = reply.status();
    let answer: Value = reply.json().await.map_err(|e| format!("speech to text at {asr}: {e}"))?;
    if !status.is_success() {
        return Err(format!("speech to text at {asr} answered {}: {answer}", status.as_u16()));
    }
    Ok(answer["text"].as_str().unwrap_or("").to_string())
}

/// Result of `speak check` for one voice.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub voice: String,
    /// Mean word error rate over the sentences, 0 = every word understood.
    pub errors: f64,
    /// Seconds of audio made per second of waiting.
    pub speed: f64,
    /// Sentences heard with more than one word in ten wrong: `(sentence, heard)`.
    pub misheard: Vec<(String, String)>,
}

/// `speak check`: each voice says each sentence, the speech-to-text server writes down
/// what it heard, and the two are compared word by word.
pub async fn check(config: &Config, only: Option<&str>, sentences: &[String]) -> Result<Vec<Checked>, String> {
    // More than one listener may be named, with commas between. A sentence counts as
    // understood as well as its best listener understood it: each transcriber has habits
    // of its own (one of ours drops a digit that is said three times in a row), and the
    // question here is what the voice said.
    let listeners: Vec<&str> = config.client.asr.split(',').map(|u| u.trim().trim_end_matches('/')).filter(|u| !u.is_empty()).collect();
    if listeners.is_empty() {
        return Err("check needs a speech-to-text server: set asr under [client], or LUMBRIDGE_SPEAK_ASR".into());
    }
    let http = http();
    // Compared against what the front is asked to say: this machine's own names included.
    let names_here = speakable(config)?;
    let names = match only {
        Some(v) => vec![v.to_string()],
        None => voices(config).await?.0,
    };
    let mut out = Vec::new();
    for name in names {
        let (mut total, mut audio_s, mut wait_s, mut misheard) = (0.0, 0.0, 0.0, Vec::new());
        for sentence in sentences {
            let asked = Instant::now();
            let wav = fetch(&http, &config.client.front, sentence, Some(&name)).await?;
            wait_s += asked.elapsed().as_secs_f64();
            let rate = wav.get(24..28).map_or(22_050, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).max(1);
            audio_s += wav.len().saturating_sub(44) as f64 / 2.0 / rate as f64;
            let said = names_here.apply(sentence);
            let (mut errors, mut heard) = (f64::INFINITY, String::new());
            for listener in &listeners {
                let text = transcribe(&http, listener, &wav).await?;
                let e = word_errors(&said, &text);
                if e < errors {
                    (errors, heard) = (e, text);
                }
                if errors == 0.0 {
                    break;
                }
            }
            total += errors;
            if errors > 0.1 {
                misheard.push((sentence.clone(), heard));
            }
        }
        out.push(Checked { voice: name, errors: total / sentences.len().max(1) as f64, speed: audio_s / wait_s.max(0.001), misheard });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_errors_compare_what_is_said_not_how_it_is_written() {
        assert_eq!(word_errors("PR 73 merged at 4:05 PM.", "PR seventy-three merged at four oh five PM"), 0.0);
        assert_eq!(word_errors("It cost $0.19.", "it cost 19 cents"), 0.0);
        assert_eq!(word_errors("red green blue gold", "red green glue gold"), 0.25);
        assert_eq!(word_errors("red green", ""), 1.0);
        // Numbers count as the same however they were said or written down.
        assert_eq!(words("The RTX 5090 and the 4080"), words("the RTX five zero nine zero and the forty eighty"));
        assert_eq!(words("4307 tests"), words("four thousand three hundred seven tests"));
        assert_eq!(words("Call 315-555-0137 now"), words("call three one five five five five oh one three seven now"));
        assert_eq!(words("Call 315-555-0137 now"), ["call", "3155550137", "now"]);
        assert_eq!(words("at 4:05 PM"), words("at four o' five PM"));
        assert_eq!(words("before 9am tomorrow"), words("before nine am tomorrow"));
        // Every repeated digit counts: a transcript that lost one of three fives is wrong.
        assert_eq!(words("Call 800-555-0199"), words("call eight zero zero five five five zero one nine nine"));
        assert_eq!(word_errors("Call 800-555-0199", "call eight zero zero five five zero one nine nine"), 0.5);
        assert_eq!(words("it took 645 ms"), words("it took six hundred forty five milliseconds"));
        assert_eq!(words("in 2026 at 9am"), words("in twenty twenty-six at nine a.m."));
        assert_eq!(words("five to ten minutes for one of them"), ["5", "to", "10", "minutes", "for", "1", "of", "them"]);
    }

    #[test]
    fn the_voice_comes_from_the_request_then_the_settings() {
        let mut config = Config::default();
        assert_eq!(voice(&config, None), None);
        assert_eq!(voice(&config, Some("Jason")), Some("Jason"));
        config.client.voice = "Leo".into();
        assert_eq!(voice(&config, None), Some("Leo"));
        assert_eq!(voice(&config, Some("Jason")), Some("Jason"));
    }
}
