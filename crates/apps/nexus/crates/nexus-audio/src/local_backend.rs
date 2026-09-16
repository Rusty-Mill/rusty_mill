//! On-device Whisper STT + OS-TTS synthesis (BL-117). Compiled in
//! only with the `local-audio` cargo feature.
//!
//! ## STT
//!
//! Wraps `whisper-rs` against a GGML model file kept under
//! `<forge>/.forge/.audio/models/ggml-<size>.bin` (the directory is
//! resolved from `AudioConfig::local_model_dir`, which
//! [`crate::AudioConfig::load`] anchors to the forge root). The
//! model is downloaded from the ggerganov HuggingFace repo on first
//! use; the download is gated by the storage subsystem's network
//! capability (audio plugin already declares `NetHttp` in
//! `audio_capabilities`). The default size is `base.en` (~140 MB)
//! per BL-117; the user can pick `tiny.en` (~75 MB) or `small.en`
//! (~466 MB) via `[audio] local_model_size = "tiny.en" | "base.en"
//! | "small.en"` in `config.toml`.
//!
//! Whisper expects 16 kHz mono f32 samples. The wire-level
//! `transcribe` payload only accepts WAV here — WebM / Opus / MP3
//! decode requires an audio framework we don't ship. Use the
//! `provider` backend for non-WAV input. A WAV at a different rate
//! or bit depth than expected gets a clear
//! [`AudioError::InvalidAudio`] error, not silent garbage.
//!
//! ## TTS
//!
//! Cross-platform shell-out, each path produces a WAV file the
//! handler reads back as bytes:
//!
//! - Linux: `espeak-ng -w <out.wav> -s 160 -- <text>`
//! - macOS: `say -o <out.wav> --data-format=LEF32@22050 -- <text>`
//!   — `say` writes a 32-bit little-endian-float PCM WAV directly
//!   when handed an explicit `--data-format`, so no AIFF→WAV
//!   transcoding is needed.
//! - Windows: PowerShell SAPI script writing a wave file.
//!
//! If the platform binary isn't on PATH the backend reports
//! [`AudioError::BackendNotEnabled`] with a hint at install
//! commands.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::backend::{
    AudioFormat, SttProvider, SynthesisOutput, TranscriptionInput, TranscriptionOutput, TtsProvider,
};
use crate::config::AudioConfig;
use crate::AudioError;

const STT_NAME: &str = "local";
const TTS_NAME: &str = "local";

/// Hard cap on the request + response time for the Whisper model
/// download. Without this a slow, hostile, or misconfigured download
/// source can block `ensure_model` (and thus the calling dispatch
/// thread) forever — `reqwest::blocking` has no timeout by default.
/// 5 minutes is generous even for the largest shipped model
/// (`small.en`, ~466 MB) on a slow connection, while still bounding
/// the wait.
const MODEL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Hard cap on the downloaded model body. The largest model size this
/// backend advertises (`small.en`) is ~466 MB; cap well above that so
/// legitimate downloads always succeed while a hostile/misconfigured
/// URL can't stream unbounded bytes into memory before we notice.
/// Mirrors the `MAX_BODY_BYTES` cap pattern used for other HTTP reads
/// in this workspace (e.g. `nexus-linkpreview`).
const MAX_MODEL_BYTES: u64 = 800 * 1024 * 1024;

/// Build the local Whisper STT backend.
#[must_use]
pub fn local_stt(cfg: AudioConfig) -> Box<dyn SttProvider> {
    Box::new(LocalWhisperStt {
        cfg,
        ctx: None,
        loaded_size: None,
    })
}

/// Build the local OS-TTS backend.
#[must_use]
pub fn local_tts(cfg: AudioConfig) -> Box<dyn TtsProvider> {
    Box::new(LocalOsTts { cfg })
}

// ─── STT ──────────────────────────────────────────────────────────────────────

struct LocalWhisperStt {
    cfg: AudioConfig,
    /// Lazy-loaded context. Whisper model load is ~1 s for `base.en`
    /// so we cache across calls; the size is remembered so a
    /// runtime config change (e.g. operator-edited TOML) reloads on
    /// next dispatch.
    ctx: Option<WhisperContext>,
    loaded_size: Option<String>,
}

impl LocalWhisperStt {
    fn model_dir(&self) -> PathBuf {
        // Anchored to the forge root by AudioConfig::load — no chdir
        // required. AudioConfig::default falls back to a relative
        // `.forge/.audio/models` so unit tests that construct a default
        // config still resolve a deterministic path.
        self.cfg.local_model_dir.clone()
    }

    fn model_filename(size: &str) -> String {
        format!("ggml-{size}.bin")
    }

    fn ensure_model(&self) -> Result<PathBuf, AudioError> {
        let size = self.cfg.local_model_size.as_str();
        let path = self.model_dir().join(Self::model_filename(size));
        if path.exists() {
            return Ok(path);
        }
        let parent = self.model_dir();
        std::fs::create_dir_all(&parent).map_err(AudioError::Io)?;
        let url = self.cfg.whisper_model_url_template.replace("{size}", size);
        tracing::info!(
            %url,
            target = %path.display(),
            "BL-117 local-audio: downloading Whisper model (first launch)"
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(MODEL_DOWNLOAD_TIMEOUT)
            .build()
            .map_err(AudioError::Network)?;
        let mut resp = client
            .get(&url)
            .send()
            .map_err(AudioError::Network)?
            .error_for_status()
            .map_err(AudioError::Network)?;
        // Reject upfront if the server announces a body over the cap,
        // before reading anything.
        if let Some(len) = resp.content_length() {
            if len > MAX_MODEL_BYTES {
                return Err(AudioError::Backend {
                    backend: STT_NAME.to_string(),
                    reason: format!(
                        "model download for '{size}' reports {len} bytes, exceeding the \
                         {MAX_MODEL_BYTES}-byte cap; refusing to download"
                    ),
                });
            }
        }
        // Also cap the actual bytes read: a server that lies about (or
        // omits) Content-Length can't be used to stream unbounded data
        // into memory. `take` bounds the reader at one byte past the
        // cap so we can tell "exactly at the cap" apart from "over the
        // cap" below rather than silently truncating a legitimate file.
        let mut buf = Vec::new();
        resp.by_ref()
            .take(MAX_MODEL_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(AudioError::Io)?;
        if buf.len() as u64 > MAX_MODEL_BYTES {
            return Err(AudioError::Backend {
                backend: STT_NAME.to_string(),
                reason: format!(
                    "model download for '{size}' exceeded the {MAX_MODEL_BYTES}-byte cap; \
                     refusing to buffer further"
                ),
            });
        }
        std::fs::write(&path, &buf).map_err(AudioError::Io)?;
        Ok(path)
    }

    fn load_ctx(&mut self) -> Result<&mut WhisperContext, AudioError> {
        let needs_reload = !matches!((&self.ctx, &self.loaded_size), (Some(_), Some(size)) if size == &self.cfg.local_model_size);
        if needs_reload {
            let path = self.ensure_model()?;
            let params = WhisperContextParameters::default();
            let ctx = WhisperContext::new_with_params(
                path.to_str().ok_or_else(|| AudioError::Backend {
                    backend: STT_NAME.to_string(),
                    reason: format!("model path is not valid UTF-8: {}", path.display()),
                })?,
                params,
            )
            .map_err(|e| AudioError::Backend {
                backend: STT_NAME.to_string(),
                reason: format!("whisper load: {e}"),
            })?;
            self.ctx = Some(ctx);
            self.loaded_size = Some(self.cfg.local_model_size.clone());
        }
        Ok(self.ctx.as_mut().expect("ctx populated above"))
    }
}

impl SttProvider for LocalWhisperStt {
    fn name(&self) -> &'static str {
        STT_NAME
    }

    fn transcribe(&mut self, input: TranscriptionInput) -> Result<TranscriptionOutput, AudioError> {
        if input.format != AudioFormat::Wav {
            return Err(AudioError::InvalidAudio(format!(
                "local Whisper backend only accepts WAV input; got {}. \
                 Re-record as 16 kHz mono WAV or use the `provider` backend.",
                input.format.as_str()
            )));
        }
        let samples = decode_wav_to_mono16k(&input.bytes)?;
        let language = input.language.clone();
        let ctx = self.load_ctx()?;
        let mut state = ctx.create_state().map_err(|e| AudioError::Backend {
            backend: STT_NAME.to_string(),
            reason: format!("whisper state: {e}"),
        })?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(lang) = language.as_deref() {
            params.set_language(Some(lang));
        }
        state
            .full(params, &samples)
            .map_err(|e| AudioError::Backend {
                backend: STT_NAME.to_string(),
                reason: format!("whisper inference: {e}"),
            })?;
        let n = state.full_n_segments();
        let mut text = String::new();
        for i in 0..n {
            let Some(seg) = state.get_segment(i) else {
                continue;
            };
            let seg_text = seg.to_str().map_err(|e| AudioError::Backend {
                backend: STT_NAME.to_string(),
                reason: format!("whisper segment {i} text: {e}"),
            })?;
            text.push_str(seg_text);
        }
        Ok(TranscriptionOutput {
            text: text.trim().to_string(),
            language,
        })
    }
}

/// Decode WAV bytes to 16 kHz mono f32 samples (Whisper's expected
/// input shape). Stereo collapses to mono via simple average; sample
/// rates other than 16 kHz are rejected because we don't ship a
/// resampler. Tell the user to re-record rather than silently giving
/// them garbage transcripts.
// Sample values are inherently bounded by the WAV bit depth / channel count,
// so the f32 precision loss here never affects the decoded audio audibly.
#[allow(clippy::cast_precision_loss)]
fn decode_wav_to_mono16k(bytes: &[u8]) -> Result<Vec<f32>, AudioError> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| AudioError::InvalidAudio(format!("wav decode header: {e}")))?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 {
        return Err(AudioError::InvalidAudio(format!(
            "local Whisper expects 16 kHz audio; got {} Hz. Re-record at 16 kHz mono.",
            spec.sample_rate
        )));
    }
    let channels = spec.channels;
    if channels == 0 {
        return Err(AudioError::InvalidAudio(
            "wav has zero channels".to_string(),
        ));
    }
    // Convert to f32 in [-1, 1].
    let mut samples_f32: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()
                .map_err(|e| AudioError::InvalidAudio(format!("wav decode: {e}")))?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| AudioError::InvalidAudio(format!("wav decode: {e}")))?,
    };
    if channels > 1 {
        // Average channels into mono.
        let c = channels as usize;
        samples_f32 = samples_f32
            .chunks_exact(c)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect();
    }
    Ok(samples_f32)
}

// ─── TTS ──────────────────────────────────────────────────────────────────────

struct LocalOsTts {
    cfg: AudioConfig,
}

impl TtsProvider for LocalOsTts {
    fn name(&self) -> &'static str {
        TTS_NAME
    }

    fn synthesize(
        &mut self,
        text: &str,
        _voice: Option<&str>,
        _format: AudioFormat,
    ) -> Result<SynthesisOutput, AudioError> {
        let _ = &self.cfg; // reserved for future per-voice config
        let tmp = tempfile::Builder::new()
            .prefix("nexus-tts-")
            .suffix(".wav")
            .tempfile()
            .map_err(AudioError::Io)?;
        let out_path = tmp.path().to_path_buf();
        run_platform_tts(text, &out_path)?;
        let bytes = std::fs::read(&out_path).map_err(AudioError::Io)?;
        // tempfile cleans up on drop, but we've already read the
        // bytes so the file can go.
        drop(tmp);
        Ok(SynthesisOutput {
            bytes,
            format: AudioFormat::Wav,
        })
    }
}

#[cfg(target_os = "linux")]
fn run_platform_tts(text: &str, out: &Path) -> Result<(), AudioError> {
    let status = Command::new("espeak-ng")
        .args(["-w"])
        .arg(out)
        .args(["-s", "160", "--"])
        .arg(text)
        .status()
        .map_err(|e| AudioError::BackendNotEnabled {
            backend: TTS_NAME.to_string(),
            reason: format!(
                "espeak-ng not found on PATH ({e}). Install with `apt install espeak-ng` \
                 / `dnf install espeak-ng` / `pacman -S espeak-ng`."
            ),
        })?;
    if !status.success() {
        return Err(AudioError::Backend {
            backend: TTS_NAME.to_string(),
            reason: format!("espeak-ng exited with {status}"),
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_platform_tts(text: &str, out: &Path) -> Result<(), AudioError> {
    let status = Command::new("say")
        .args(["-o"])
        .arg(out)
        .args(["--data-format=LEF32@22050"])
        .args(["--"])
        .arg(text)
        .status()
        .map_err(|e| AudioError::BackendNotEnabled {
            backend: TTS_NAME.to_string(),
            reason: format!("`say` not found on PATH ({e})"),
        })?;
    if !status.success() {
        return Err(AudioError::Backend {
            backend: TTS_NAME.to_string(),
            reason: format!("say exited with {status}"),
        });
    }
    Ok(())
}

/// Quote `s` as a PowerShell single-quoted string literal. Single-quoted
/// strings in PowerShell are true literals: unlike double-quoted strings,
/// they never interpolate variables (`$foo`) or subexpressions (`$(...)`)
/// — the only special character is the quote delimiter itself, escaped by
/// doubling it. Used only for the paths embedded in the SAPI script below;
/// the synthesized *text* never goes through this (or any) script-string
/// interpolation path — see `run_platform_tts`.
#[cfg(target_os = "windows")]
fn powershell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push('\'');
        }
        out.push(ch);
    }
    out.push('\'');
    out
}

/// Build the PowerShell SAPI script. `text_file` is a path to a file
/// already holding the text to speak — it is read back as *data*
/// (`Get-Content -Raw`), never interpolated into the script string. This
/// is deliberate: embedding untrusted text into a PowerShell `-Command`
/// string (even after quote-escaping) is vulnerable to `$(...)`
/// subexpression injection inside double-quoted contexts, letting
/// attacker-controlled text (e.g. text read aloud from an untrusted
/// source) execute arbitrary PowerShell. Routing it through a file read
/// instead means the text can never be reinterpreted as code, regardless
/// of its contents.
#[cfg(target_os = "windows")]
fn build_windows_tts_script(out: &Path, text_file: &Path) -> String {
    format!(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $s.SetOutputToWaveFile({out}); \
         $text = Get-Content -LiteralPath {text_file} -Raw -Encoding UTF8; \
         $s.Speak($text);",
        out = powershell_single_quote(&out.display().to_string()),
        text_file = powershell_single_quote(&text_file.display().to_string()),
    )
}

#[cfg(target_os = "windows")]
fn run_platform_tts(text: &str, out: &Path) -> Result<(), AudioError> {
    // Text is written to a temp file and read back by the script as data
    // (see `build_windows_tts_script`) instead of being concatenated into
    // the `-Command` string, so it can never be reinterpreted as
    // PowerShell code no matter what it contains.
    let text_file = tempfile::Builder::new()
        .prefix("nexus-tts-text-")
        .suffix(".txt")
        .tempfile()
        .map_err(AudioError::Io)?;
    std::fs::write(text_file.path(), text).map_err(AudioError::Io)?;
    let script = build_windows_tts_script(out, text_file.path());
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|e| AudioError::BackendNotEnabled {
            backend: TTS_NAME.to_string(),
            reason: format!("powershell not found on PATH ({e})"),
        })?;
    if !status.success() {
        return Err(AudioError::Backend {
            backend: TTS_NAME.to_string(),
            reason: format!("powershell exited with {status}"),
        });
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn run_platform_tts(_text: &str, _out: &Path) -> Result<(), AudioError> {
    Err(AudioError::BackendNotEnabled {
        backend: TTS_NAME.to_string(),
        reason: "local TTS shell-out not implemented for this platform".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_wav_input() {
        let mut stt = LocalWhisperStt {
            cfg: AudioConfig::default(),
            ctx: None,
            loaded_size: None,
        };
        let err = stt
            .transcribe(TranscriptionInput {
                bytes: vec![1, 2, 3],
                format: AudioFormat::Webm,
                language: None,
            })
            .unwrap_err();
        match err {
            AudioError::InvalidAudio(msg) => assert!(msg.contains("WAV")),
            other => panic!("expected InvalidAudio, got {other:?}"),
        }
    }

    #[test]
    fn rejects_wrong_sample_rate() {
        // Build a tiny 48 kHz WAV header and feed it in.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Vec::new();
        {
            let mut w = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
            for _ in 0..100 {
                w.write_sample(0_i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let err = decode_wav_to_mono16k(&buf).unwrap_err();
        match err {
            AudioError::InvalidAudio(msg) => assert!(msg.contains("16 kHz")),
            other => panic!("expected InvalidAudio, got {other:?}"),
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn decodes_16khz_mono_wav() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Vec::new();
        {
            let mut w = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
            for i in 0..1600 {
                let v = ((i as f32) / 100.0).sin();
                w.write_sample((v * f32::from(i16::MAX)) as i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let samples = decode_wav_to_mono16k(&buf).unwrap();
        assert_eq!(samples.len(), 1600);
    }

    #[test]
    fn collapses_stereo_to_mono() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Vec::new();
        {
            let mut w = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
            for _ in 0..800 {
                w.write_sample(1000_i16).unwrap(); // L
                w.write_sample(-500_i16).unwrap(); // R → averages to 250
            }
            w.finalize().unwrap();
        }
        let samples = decode_wav_to_mono16k(&buf).unwrap();
        assert_eq!(samples.len(), 800);
        // Both channels are constant, so the average is constant.
        // 1000 + (-500) = 500, /2 = 250; /max ~= 250 / 32768 ~ 0.00763
        let expected = f32::midpoint(1000.0_f32, -500.0_f32) / 32768.0;
        assert!((samples[0] - expected).abs() < 1e-4);
    }

    // ─── BL-117 defect fixes: PowerShell injection + unbounded download ───

    /// Regression for the PowerShell `$(...)` subexpression-injection RCE:
    /// before the fix, `run_platform_tts` built the `-Command` string by
    /// double-quote-escaping `text` and splicing it straight into a
    /// double-quoted `Speak("...")` literal, where `$(...)` is evaluated as
    /// a live subexpression. The fixed script routes text through a file
    /// (read back via `Get-Content -Raw`) instead, so none of it — payload
    /// included — ever appears in the command text at all.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_tts_script_never_embeds_text_for_interpolation() {
        let payload = "hello $(New-Item -ItemType File -Path 'C:\\pwned.txt' -Force) world";
        let out = Path::new(r"C:\out.wav");
        let text_file = Path::new(r"C:\Users\x\AppData\Local\Temp\nexus-tts-text-abcd.txt");
        let script = build_windows_tts_script(out, text_file);
        assert!(
            !script.contains(payload),
            "payload text must never be interpolated into the PowerShell command string: {script}"
        );
        assert!(
            !script.contains("New-Item"),
            "no fragment of the payload should leak into the command string: {script}"
        );
        assert!(
            script.contains("Get-Content") && script.contains("-Raw"),
            "text must be read back from disk as data, not embedded in the script: {script}"
        );
    }

    /// End-to-end proof mirroring the manual repro used to confirm the
    /// vulnerability: a payload shaped like a PowerShell subexpression
    /// that, if ever evaluated as code, creates a marker file. Verified by
    /// hand against the pre-fix escaping logic (`text.replace('"',
    /// "`\"")` spliced into a double-quoted `Speak("...")` literal) that
    /// the marker file *is* created there. Post-fix, `run_platform_tts`
    /// only ever hands PowerShell the text via `Get-Content -Raw` on a
    /// temp file, so the subexpression is never parsed as code.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_tts_treats_subexpression_payload_as_inert_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("marker_proof.txt");
        let payload = format!(
            "hello $(New-Item -ItemType File -Path '{}' -Force) world",
            marker.display()
        );
        let out_wav = dir.path().join("out.wav");
        run_platform_tts(&payload, &out_wav).expect("TTS shell-out should succeed");
        assert!(
            !marker.exists(),
            "subexpression payload must not execute: marker file must not exist"
        );
        assert!(
            out_wav.exists() && std::fs::metadata(&out_wav).unwrap().len() > 0,
            "TTS should still produce audio for the literal (unexecuted) text"
        );
    }

    /// Regression for the unbounded, timeout-less `ensure_model` download:
    /// before the fix, `ensure_model` called
    /// `reqwest::blocking::get(&url).bytes()` with no client timeout and no
    /// size cap, so a hostile/misconfigured download source that declares
    /// an oversized `Content-Length` and then never sends the body would
    /// block the calling thread indefinitely. Post-fix, the declared
    /// length is checked against `MAX_MODEL_BYTES` before any body byte is
    /// read (or waited for), so this returns an error almost immediately
    /// instead of hanging.
    #[test]
    fn ensure_model_rejects_oversized_download_without_hanging() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::sync::mpsc;
        use std::time::Duration as StdDuration;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = Read::read(&mut stream, &mut buf);
                // Declare a body far larger than any real Whisper model
                // (and far larger than MAX_MODEL_BYTES) via Content-Length,
                // then hold the connection open without ever sending that
                // many bytes — a hostile/misconfigured server stalling the
                // caller.
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    10u64 * 1024 * 1024 * 1024 // 10 GiB, never actually sent
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                // Stall: a pre-fix caller with no cap/timeout would block
                // here indefinitely waiting for the declared body.
                std::thread::sleep(StdDuration::from_secs(120));
            }
        });

        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = AudioConfig {
            local_model_dir: dir.path().to_path_buf(),
            local_model_size: "tiny.en".to_string(),
            whisper_model_url_template: format!("http://{addr}/ggml-{{size}}.bin"),
            ..AudioConfig::default()
        };
        let stt = LocalWhisperStt {
            cfg,
            ctx: None,
            loaded_size: None,
        };

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = stt.ensure_model();
            let _ = tx.send(result.is_err());
        });
        let got_err = rx.recv_timeout(StdDuration::from_secs(15)).expect(
            "ensure_model must reject an oversized/hostile download within a bounded \
             time instead of hanging indefinitely",
        );
        assert!(got_err, "oversized download must be rejected, not accepted");
    }
}
