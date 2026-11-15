//! Settings: built-in defaults, then a TOML file, then the environment
//! (`LUMBRIDGE_SPEAK_*`). The defaults expect a Magpie server on loopback `:8808`; the front
//! listens on loopback `:8813`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Address and port to listen on.
    pub listen: String,
    /// Which model adapter: `magpie` or `mock`.
    pub adapter: String,
    /// Voice used when the caller names none, or one the model does not have.
    pub default_voice: String,
    /// What `/health` and the voice list report as `default`. It selects nothing (the
    /// voice used is `default_voice`); callers may show it.
    pub reported_default_voice: String,
    /// The `ffmpeg` program, for every format except `wav`.
    pub ffmpeg: String,
    /// Write numbers, money, times, versions and paths as words before the model sees
    /// them. A request can turn it off for itself with `"normalize": false`.
    pub normalize: bool,
    /// How this installation's own names are said: written form → spoken form.
    pub say: BTreeMap<String, String>,
    /// Patterns (regular expressions) for things that are never read aloud.
    pub drop: Vec<String>,
    /// Loudness: the answer is scaled so its loudest sample sits at this level
    /// (0 to 1). 0 leaves the model's own level, which is quiet (peak near 0.5).
    pub level: f32,
    pub magpie: Magpie,
    pub client: Client,
}

/// Settings of the `speak` command (`lumbridge-speak say` and friends).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Client {
    /// The front to ask for audio.
    pub front: String,
    /// Voice to ask for; empty = the front's default.
    pub voice: String,
    /// Playback speed, 0.5 to 2.
    pub rate: f32,
    /// Speech-to-text servers (`/v1/audio/transcriptions`) for `speak check`; several are
    /// separated by commas.
    pub asr: String,
    /// Other machines that speak for us: `speak --on NAME`.
    pub targets: BTreeMap<String, Target>,
}

/// A machine with an announce endpoint (`POST <url>/say`, bearer token).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub url: String,
    /// File holding the bearer token. `~/` is the home folder.
    #[serde(default)]
    pub token_file: String,
}

impl Default for Client {
    fn default() -> Self {
        Self { front: "http://127.0.0.1:8813".into(), voice: String::new(), rate: 1.0, asr: String::new(), targets: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Magpie {
    pub url: String,
    pub model: String,
    /// Sample rate of the model's output.
    pub rate: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8813".into(),
            adapter: "magpie".into(),
            default_voice: "Leo".into(),
            reported_default_voice: "default".into(),
            ffmpeg: "ffmpeg".into(),
            normalize: true,
            say: BTreeMap::new(),
            drop: Vec::new(),
            level: 0.0,
            magpie: Magpie::default(),
            client: Client::default(),
        }
    }
}

impl Default for Magpie {
    fn default() -> Self {
        Self { url: "http://127.0.0.1:8808".into(), model: "magpietts".into(), rate: 22_050 }
    }
}

impl Config {
    /// `file` is the TOML file, if any. `env` looks up an environment variable; an empty
    /// value counts as not set.
    pub fn load(file: Option<&Path>, env: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut c = match file {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
                toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            None => Config::default(),
        };
        let get = |name: &str| env(&format!("LUMBRIDGE_SPEAK_{name}")).filter(|v| !v.is_empty());
        if let Some(v) = get("LISTEN") {
            c.listen = v;
        }
        if let Some(v) = get("ADAPTER") {
            c.adapter = v;
        }
        if let Some(v) = get("DEFAULT_VOICE") {
            c.default_voice = v;
        }
        if let Some(v) = get("REPORTED_DEFAULT_VOICE") {
            c.reported_default_voice = v;
        }
        if let Some(v) = get("FFMPEG") {
            c.ffmpeg = v;
        }
        if let Some(v) = get("NORMALIZE") {
            c.normalize = !matches!(v.as_str(), "0" | "false" | "no" | "off");
        }
        if let Some(v) = get("LEVEL") {
            c.level = v.parse().map_err(|_| format!("LUMBRIDGE_SPEAK_LEVEL: not a number: {v}"))?;
        }
        if let Some(v) = get("FRONT") {
            c.client.front = v;
        }
        if let Some(v) = get("VOICE") {
            c.client.voice = v;
        }
        if let Some(v) = get("ASR") {
            c.client.asr = v;
        }
        if let Some(v) = get("MAGPIE_URL") {
            c.magpie.url = v;
        }
        if let Some(v) = get("MAGPIE_MODEL") {
            c.magpie.model = v;
        }
        if let Some(v) = get("MAGPIE_RATE") {
            c.magpie.rate = v.parse().map_err(|_| format!("LUMBRIDGE_SPEAK_MAGPIE_RATE: not a number: {v}"))?;
        }
        if !matches!(c.adapter.as_str(), "magpie" | "mock") {
            return Err(format!("adapter must be magpie or mock, not {}", c.adapter));
        }
        if !(0.0..=1.0).contains(&c.level) {
            return Err(format!("level must be between 0 and 1, not {}", c.level));
        }
        if !(0.5..=2.0).contains(&c.client.rate) {
            return Err(format!("client rate must be between 0.5 and 2, not {}", c.client.rate));
        }
        c.client.front = c.client.front.trim_end_matches('/').to_string();
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_live_values_on_another_port() {
        let c = Config::load(None, |_| None).unwrap();
        assert_eq!(c.listen, "127.0.0.1:8813");
        assert_eq!((c.adapter.as_str(), c.default_voice.as_str()), ("magpie", "Leo"));
        assert_eq!(c.magpie, Magpie { url: "http://127.0.0.1:8808".into(), model: "magpietts".into(), rate: 22_050 });
    }

    #[test]
    fn file_then_environment() {
        let dir = std::env::temp_dir().join(format!("lumbridge-speak-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("speak.toml");
        std::fs::write(&path, "listen = \"0.0.0.0:8803\"\ndefault_voice = \"Aria\"\n[magpie]\nrate = 44100\n").unwrap();
        let env = |name: &str| match name {
            "LUMBRIDGE_SPEAK_DEFAULT_VOICE" => Some("John".to_string()),
            "LUMBRIDGE_SPEAK_MAGPIE_URL" => Some(String::new()), // empty = not set
            _ => None,
        };
        let c = Config::load(Some(&path), env).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(c.listen, "0.0.0.0:8803"); // from the file
        assert_eq!(c.default_voice, "John"); // the environment wins over the file
        assert_eq!(c.magpie.rate, 44_100);
        assert_eq!(c.magpie.url, "http://127.0.0.1:8808"); // default kept
    }

    #[test]
    fn mistakes_are_reported() {
        let env = |name: &str| (name == "LUMBRIDGE_SPEAK_ADAPTER").then(|| "voxcpm".to_string());
        assert!(Config::load(None, env).unwrap_err().contains("magpie or mock"));
        let env = |name: &str| (name == "LUMBRIDGE_SPEAK_MAGPIE_RATE").then(|| "fast".to_string());
        assert!(Config::load(None, env).unwrap_err().contains("not a number"));
    }
}
