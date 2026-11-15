use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use lumbridge_speak::adapter::{Magpie, Mock, Synth};
use lumbridge_speak::client::{self, Say};
use lumbridge_speak::config::Config;
use lumbridge_speak::server::{router, App};

const USAGE: &str = "\
lumbridge-speak: text to speech. The HTTP front, and the `speak` command.

  lumbridge-speak [serve]            run the HTTP front
  lumbridge-speak say TEXT...        speak on this machine (TEXT `-` or none: read stdin)
      --voice NAME   --rate N        which voice, how fast (0.5 to 2)
      --on MACHINE   --from NAME     speak on another machine from [client.targets]
      --out FILE                     write a WAV file instead of playing
  lumbridge-speak voices             list the voices
  lumbridge-speak demo [TEXT...]     hear every voice, one after another
  lumbridge-speak check [--voice NAME] [TEXT...]
                                     speak, transcribe, compare: can the voices be understood?
  lumbridge-speak text TEXT...       print the words that would be spoken
  lumbridge-speak stop               stop what is playing on this machine

Installed under the name `speak`, the command is `say` unless the first word is one of
the others: `speak Hello there`, `speak voices`, `echo Hello | speak`.

Options for every command: --config FILE (TOML; also $LUMBRIDGE_SPEAK_CONFIG, then
~/.config/lumbridge-speak/config.toml if it exists), --print-config.
Settings: built-in defaults, then the file, then LUMBRIDGE_SPEAK_* in the environment.
See lumbridge-speak.example.toml.";

const COMMANDS: [&str; 7] = ["serve", "say", "voices", "demo", "check", "text", "stop"];

/// A command line, taken apart.
#[derive(Debug, Default, PartialEq)]
struct Asked {
    command: String,
    words: Vec<String>,
    config: Option<PathBuf>,
    print_config: bool,
    help: bool,
    say: Say,
}

/// `as_speak`: the program was started under the name `speak`.
fn parse(args: impl IntoIterator<Item = String>, as_speak: bool) -> Result<Asked, String> {
    let mut asked = Asked::default();
    let mut args = args.into_iter();
    let mut literal = false; // after `--`, everything is text
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            _ if literal => asked.words.push(arg),
            "--" => literal = true,
            "--config" => asked.config = Some(PathBuf::from(value("--config")?)),
            "--print-config" => asked.print_config = true,
            "-h" | "--help" => asked.help = true,
            "--voice" | "-v" => asked.say.voice = Some(value("--voice")?),
            "--rate" | "-r" => {
                let v = value("--rate")?;
                let rate: f32 = v.parse().map_err(|_| format!("--rate: not a number: {v}"))?;
                if !(0.5..=2.0).contains(&rate) {
                    return Err(format!("--rate must be between 0.5 and 2, not {v}"));
                }
                asked.say.rate = Some(rate);
            }
            "--on" => asked.say.on = Some(value("--on")?),
            "--from" => asked.say.from = Some(value("--from")?),
            "--out" | "-o" => asked.say.out = Some(PathBuf::from(value("--out")?)),
            "-" => asked.words.push(arg),
            other if other.starts_with('-') && other.len() > 1 && asked.words.is_empty() && asked.command.is_empty() => {
                return Err(format!("unknown option {other}"))
            }
            other if asked.command.is_empty() && asked.words.is_empty() && COMMANDS.contains(&other) => {
                asked.command = other.to_string()
            }
            _ => asked.words.push(arg),
        }
    }
    if asked.command.is_empty() {
        // `speak` with or without words speaks; `lumbridge-speak` without words serves.
        asked.command = if as_speak || !asked.words.is_empty() { "say" } else { "serve" }.to_string();
        if !as_speak && !asked.words.is_empty() {
            return Err(format!("unknown command {}", asked.words[0]));
        }
    }
    Ok(asked)
}

fn main() -> ExitCode {
    let mut args = std::env::args();
    let as_speak = args.next().is_some_and(|name| PathBuf::from(name).file_name().is_some_and(|n| n == "speak"));
    let asked = match parse(args, as_speak) {
        Ok(asked) => asked,
        Err(e) => return fail(&format!("{e}\n\n{USAGE}")),
    };
    if asked.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let file = asked.config.clone().or(env("LUMBRIDGE_SPEAK_CONFIG").map(PathBuf::from)).or_else(|| {
        let own = PathBuf::from(env("HOME")?).join(".config/lumbridge-speak/config.toml");
        own.is_file().then_some(own)
    });
    let config = match Config::load(file.as_deref(), |name| std::env::var(name).ok()) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    if asked.print_config {
        println!("{config:#?}");
        return ExitCode::SUCCESS;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let done = runtime.block_on(run(asked, &config));
    // A player that was still starting must not outlive us.
    runtime.shutdown_background();
    match done {
        Ok(code) => code,
        Err(e) => fail(&e),
    }
}

async fn run(asked: Asked, config: &Config) -> Result<ExitCode, String> {
    let text = || -> Result<String, String> {
        if asked.words.is_empty() || asked.words == ["-"] {
            let mut all = String::new();
            std::io::stdin().read_to_string(&mut all).map_err(|e| format!("stdin: {e}"))?;
            Ok(all)
        } else {
            Ok(asked.words.join(" "))
        }
    };
    match asked.command.as_str() {
        "serve" => match config.adapter.as_str() {
            "mock" => serve(Mock::new(config.magpie.rate), config).await?,
            _ => serve(Magpie::new(&config.magpie.url, &config.magpie.model, config.magpie.rate), config).await?,
        },
        "say" => client::say(config, &Say { text: text()?, ..asked.say }).await?,
        "text" => println!("{}", client::speakable(config)?.apply(&text()?)),
        "stop" => {
            if !client::stop() {
                println!("nothing is playing");
            }
        }
        "voices" => {
            let (names, default) = client::voices(config).await?;
            for name in names {
                println!("{name}");
            }
            if !default.is_empty() {
                println!("default: {default}");
            }
        }
        "demo" => {
            let sentence =
                if asked.words.is_empty() { "I can read your updates, your answers, and anything you select.".to_string() } else { asked.words.join(" ") };
            client::demo(config, &sentence, asked.say.rate).await?;
        }
        "check" => {
            let sentences: Vec<String> = match asked.words.is_empty() {
                true => client::CHECK_SENTENCES.iter().map(|s| s.to_string()).collect(),
                false => vec![asked.words.join(" ")],
            };
            let results = client::check(config, asked.say.voice.as_deref(), &sentences).await?;
            let mut good = true;
            for r in &results {
                println!(
                    "{:<8} {:>5.1}% of words understood   {:>4.0}x faster than real time",
                    r.voice,
                    (1.0 - r.errors).max(0.0) * 100.0,
                    r.speed
                );
                for (said, heard) in &r.misheard {
                    println!("         said:  {said}\n         heard: {heard}");
                }
                good &= r.errors <= 0.1;
            }
            if !good {
                return Ok(ExitCode::FAILURE);
            }
        }
        other => return Err(format!("unknown command {other}")),
    }
    Ok(ExitCode::SUCCESS)
}

async fn serve<S: Synth>(synth: S, config: &Config) -> Result<(), String> {
    let app = Arc::new(App {
        synth,
        default_voice: config.default_voice.clone(),
        reported_default_voice: config.reported_default_voice.clone(),
        ffmpeg: config.ffmpeg.clone(),
        speakable: if config.normalize { Some(client::speakable(config)?) } else { None },
        level: config.level,
    });
    let listener =
        tokio::net::TcpListener::bind(&config.listen).await.map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
    eprintln!(
        "lumbridge-speak listening on {} (adapter {}, model at {}, numbers as words: {})",
        config.listen,
        config.adapter,
        app.synth.location(),
        if config.normalize { "on" } else { "off" }
    );
    axum::serve(listener, router(app)).await.map_err(|e| e.to_string())
}

fn fail(message: &str) -> ExitCode {
    eprintln!("lumbridge-speak: {message}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(line: &str, as_speak: bool) -> Result<Asked, String> {
        parse(line.split_whitespace().map(str::to_string), as_speak)
    }

    #[test]
    fn no_words_serves_and_speak_speaks() {
        assert_eq!(asked("", false).unwrap().command, "serve");
        assert_eq!(asked("--config x.toml", false).unwrap().config, Some(PathBuf::from("x.toml")));
        assert_eq!(asked("", true).unwrap().command, "say");
        let a = asked("Hello there --voice Jason", true).unwrap();
        assert_eq!((a.command.as_str(), a.words.join(" ").as_str()), ("say", "Hello there"));
        assert_eq!(a.say.voice.as_deref(), Some("Jason"));
    }

    #[test]
    fn commands_and_their_options() {
        let a = asked("say --on laptop --from Sam The build is done", false).unwrap();
        assert_eq!((a.command.as_str(), a.words.len()), ("say", 4));
        assert_eq!((a.say.on.as_deref(), a.say.from.as_deref()), (Some("laptop"), Some("Sam")));
        assert_eq!(asked("voices", true).unwrap().command, "voices");
        assert_eq!(asked("check --voice Leo", false).unwrap().say.voice.as_deref(), Some("Leo"));
        // A command word later in the text is text, and so is anything after `--`.
        let a = asked("please stop -- --now", true).unwrap();
        assert_eq!((a.command.as_str(), a.words.join(" ").as_str()), ("say", "please stop --now"));
        assert_eq!(asked("say - --rate 1.2", false).unwrap().say.rate, Some(1.2));
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(asked("--rate fast hello", true).unwrap_err().contains("not a number"));
        assert!(asked("--rate 9 hello", true).unwrap_err().contains("between"));
        assert!(asked("--voice", true).unwrap_err().contains("needs a value"));
        assert!(asked("--loud hello", true).unwrap_err().contains("unknown option"));
        assert!(asked("hello", false).unwrap_err().contains("unknown command"));
    }
}
