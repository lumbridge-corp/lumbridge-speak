//! Turning the model's raw samples (mono, 16-bit, little-endian) into the format the
//! caller asked for. WAV is written here; everything else goes through `ffmpeg`, with
//! the same arguments the Python front used.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Raw `pcm` answers are always at this rate: the OpenAI rule for raw audio.
pub const PCM_RATE: u32 = 24_000;
/// Silence put between two pieces of one request, in seconds.
pub const GAP_SECONDS: f64 = 0.15;

/// `response_format` values and their content types.
pub fn content_type(format: &str) -> Option<&'static str> {
    Some(match format {
        "mp3" => "audio/mpeg",
        "opus" => "audio/ogg",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "pcm" => "audio/pcm",
        _ => return None,
    })
}

/// The silence between two pieces, as samples at `rate`.
pub fn gap(rate: u32) -> Vec<u8> {
    vec![0u8; (rate as f64 * GAP_SECONDS) as usize * 2]
}

/// Scales mono 16-bit samples so the loudest one sits at `level` (0 to 1) of full scale.
/// Near-silence is left alone, and nothing is made more than four times louder.
pub fn set_level(pcm: &mut [u8], level: f32) {
    if level <= 0.0 {
        return;
    }
    let peak = pcm.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs()).max().unwrap_or(0);
    if peak < 328 {
        return; // below 1% of full scale: silence or noise
    }
    let gain = (level.min(1.0) * 32767.0 / peak as f32).min(4.0);
    for s in pcm.chunks_exact_mut(2) {
        let v = (i16::from_le_bytes([s[0], s[1]]) as f32 * gain).round().clamp(-32768.0, 32767.0) as i16;
        s.copy_from_slice(&v.to_le_bytes());
    }
}

/// A WAV file (44-byte header) around mono 16-bit samples.
pub fn wav(pcm: &[u8], rate: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Encodes samples at `rate` as `format` (one of the names [`content_type`] knows).
pub async fn encode(ffmpeg: &str, pcm: Vec<u8>, format: &str, rate: u32) -> Result<Vec<u8>, String> {
    let rate_arg = rate.to_string();
    let input = ["-hide_banner", "-loglevel", "error", "-f", "s16le", "-ar", &rate_arg, "-ac", "1", "-i", "pipe:0"];
    let pcm_rate = PCM_RATE.to_string();
    let output: Vec<&str> = match format {
        "pcm" if rate == PCM_RATE => return Ok(pcm),
        "pcm" => vec!["-ar", &pcm_rate, "-f", "s16le", "pipe:1"],
        "wav" => return Ok(wav(&pcm, rate)),
        "mp3" => vec!["-f", "mp3", "pipe:1"],
        "opus" => vec!["-c:a", "libopus", "-b:a", "48k", "-f", "ogg", "pipe:1"],
        "aac" => vec!["-f", "adts", "pipe:1"],
        "flac" => vec!["-f", "flac", "pipe:1"],
        other => return Err(format!("unsupported response_format {other}")),
    };

    let mut child = Command::new(ffmpeg)
        .args(input)
        .args(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    // Feed and drain at the same time, or a full pipe stalls both sides.
    let feed = tokio::spawn(async move {
        let _ = stdin.write_all(&pcm).await;
        let _ = stdin.shutdown().await;
    });
    let done = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| "ffmpeg: timed out after 60 seconds".to_string())?
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let _ = feed.await;
    if !done.status.success() {
        let shown = &done.stderr[..done.stderr.len().min(200)];
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(shown)));
    }
    Ok(done.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_the_one_the_python_front_wrote() {
        // Header of a real answer from the Python front (2026-11-09), 69,632 bytes of samples at 22050 Hz.
        let observed: [u8; 44] = [
            0x52, 0x49, 0x46, 0x46, 0x24, 0x10, 0x01, 0x00, 0x57, 0x41, 0x56, 0x45, 0x66, 0x6d, 0x74, 0x20, 0x10, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x22, 0x56, 0x00, 0x00, 0x44, 0xac, 0x00, 0x00, 0x02, 0x00, 0x10, 0x00,
            0x64, 0x61, 0x74, 0x61, 0x00, 0x10, 0x01, 0x00,
        ];
        let out = wav(&vec![0u8; 69_632], 22_050);
        assert_eq!(out.len(), 69_676);
        assert_eq!(out[..44], observed);
    }

    #[test]
    fn level_scales_to_the_peak_and_leaves_silence_alone() {
        let mut pcm: Vec<u8> = [8000i16, -16000, 400].iter().flat_map(|v| v.to_le_bytes()).collect();
        set_level(&mut pcm, 0.9);
        let out: Vec<i16> = pcm.chunks(2).map(|s| i16::from_le_bytes([s[0], s[1]])).collect();
        assert_eq!(out, [14745, -29490, 737]);
        let mut quiet: Vec<u8> = [100i16, -200].iter().flat_map(|v| v.to_le_bytes()).collect();
        let before = quiet.clone();
        set_level(&mut quiet, 0.9);
        assert_eq!(quiet, before);
        let mut soft: Vec<u8> = [1000i16].iter().flat_map(|v| v.to_le_bytes()).collect();
        set_level(&mut soft, 1.0);
        assert_eq!(i16::from_le_bytes([soft[0], soft[1]]), 4000); // at most four times louder
        let mut off = before.clone();
        set_level(&mut off, 0.0);
        assert_eq!(off, before);
    }

    #[test]
    fn gap_is_150_ms_rounded_down_to_a_sample() {
        assert_eq!(gap(22_050).len(), 3307 * 2); // int(22050 * 0.15) in the Python front
        assert_eq!(gap(48_000).len(), 7200 * 2);
    }
}
