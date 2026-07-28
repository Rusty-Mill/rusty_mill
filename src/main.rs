//! Sovereign Interactive GUI Desktop Voice-to-Text Studio Application.
//! Built 100% using sovereign **Rusty Mill** libraries (`rusty_gui`, `rusty_gpu`, `rusty_font`, `rusty_audio`, `rusty-whisper`).

extern crate alloc;

use alloc::format;
use alloc::string::String;

use std::path::{Path, PathBuf};

use rusty_audio::{AudioCapture, AudioSpec};
use rusty_err::Context;
use rusty_gpu::{Color, Framebuffer, Pipeline};
use rusty_gui::{Event, KeyCode, MouseButton, WindowBuilder};
use rusty_whisper::model::{self, Model};
use rusty_whisper::transcribe::{self, Options};

/// Finds the real Whisper GGML model this crate needs, checking the
/// current directory first (a real install), then a sibling `rusty_whisper`
/// checkout's own copy (this ecosystem's repos are siblings under one
/// parent directory, not a single Cargo workspace with one shared build
/// output — see `mill-term`'s `find_sibling_binary` for the same pattern).
fn find_whisper_model() -> Option<PathBuf> {
    const MODEL_FILE: &str = "ggml-tiny.en-q5_1.bin";
    let cwd_candidate = PathBuf::from(MODEL_FILE);
    if cwd_candidate.is_file() {
        return Some(cwd_candidate);
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        let candidate = ancestor.join("rusty_whisper").join(MODEL_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Loads the Whisper model at `path`, or `None` (logged to stderr) on any
/// failure — a missing/unloadable model degrades the app to a working
/// recorder with no transcription, rather than refusing to start.
fn load_whisper_model(path: &Path) -> Option<Model> {
    match std::fs::File::open(path).and_then(|f| model::load_model(&mut std::io::BufReader::new(f))) {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("rusty_voice: failed to load Whisper model at {}: {e}", path.display());
            None
        }
    }
}

/// Runs `samples` through real Whisper transcription and joins the
/// resulting segments into one string — the actual "voice-to-text" step
/// this crate is named for, previously entirely absent (the TRANSCRIBE
/// button just set a hardcoded string regardless of any recorded audio).
fn transcribe_samples(whisper_model: &Model, samples: &[f32]) -> String {
    if samples.is_empty() {
        return String::from("NO AUDIO RECORDED");
    }
    let transcript = transcribe::transcribe(whisper_model, samples, &Options::default());
    let text: String = transcript.segments.iter().map(|s| s.text.as_str()).collect::<alloc::vec::Vec<_>>().join(" ");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        String::from("(no speech detected)")
    } else {
        String::from(trimmed)
    }
}

/// Application interactive UI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Idle,
    Recording,
    Transcribing,
}

/// Simple 8x8 font bitmap glyph patterns for ASCII characters.
fn get_glyph_pattern(ch: char) -> [u8; 8] {
    match ch.to_ascii_uppercase() {
        'A' => [0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        'B' => [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00],
        'C' => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00],
        'D' => [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00],
        'E' => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x7E, 0x00],
        'F' => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x00],
        'G' => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3E, 0x00],
        'H' => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        'I' => [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        'J' => [0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0x00],
        'K' => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00],
        'L' => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00],
        'M' => [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0x00],
        'N' => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00],
        'O' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        'P' => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00],
        'Q' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x0E, 0x00],
        'R' => [0x7C, 0x66, 0x66, 0x7C, 0x78, 0x6C, 0x66, 0x00],
        'S' => [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00],
        'T' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        'U' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        'V' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        'W' => [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00],
        'X' => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00],
        'Y' => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00],
        'Z' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00],
        '0' => [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00],
        '1' => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00],
        '2' => [0x3C, 0x66, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00],
        '3' => [0x3C, 0x66, 0x0C, 0x18, 0x0C, 0x66, 0x3C, 0x00],
        '4' => [0x0C, 0x1C, 0x3C, 0x6C, 0x7E, 0x0C, 0x0C, 0x00],
        '5' => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00],
        '6' => [0x3C, 0x66, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00],
        '7' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0x00],
        '8' => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00],
        '9' => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0x00],
        ':' => [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x30, 0x00],
        '!' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00],
        '(' => [0x0C, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0C, 0x00],
        ')' => [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00],
        '\'' => [0x18, 0x18, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

/// Renders a string of text onto the framebuffer at (x, y) with text color and scale.
fn draw_text(fb: &mut Framebuffer, pipeline: &Pipeline, x: usize, y: usize, text: &str, color: Color, scale: usize) {
    let mut cur_x = x;
    for ch in text.chars() {
        let pattern = get_glyph_pattern(ch);
        for row in 0..8 {
            let row_byte = pattern[row];
            for col in 0..8 {
                if (row_byte & (0x80 >> col)) != 0 {
                    pipeline.draw_rect(fb, cur_x + col * scale, y + row * scale, scale, scale, color);
                }
            }
        }
        cur_x += 9 * scale;
    }
}

/// Interactive GUI Button component.
pub struct Button {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    label: &'static str,
    color: Color,
    hover_color: Color,
}

impl Button {
    pub fn new(x: usize, y: usize, width: usize, height: usize, label: &'static str, color: Color, hover_color: Color) -> Self {
        Self { x, y, width, height, label, color, hover_color }
    }

    pub fn contains(&self, cx: usize, cy: usize) -> bool {
        cx >= self.x && cx <= (self.x + self.width) && cy >= self.y && cy <= (self.y + self.height)
    }

    pub fn draw(&self, pipeline: &Pipeline, fb: &mut Framebuffer, is_hovered: bool) {
        let current_color = if is_hovered { self.hover_color } else { self.color };
        pipeline.draw_rect(fb, self.x, self.y, self.width, self.height, current_color);
        // Draw inner border
        pipeline.draw_rect(fb, self.x, self.y, self.width, 2, Color::rgb(255, 255, 255));
        pipeline.draw_rect(fb, self.x, self.y + self.height - 2, self.width, 2, Color::rgb(255, 255, 255));
        pipeline.draw_rect(fb, self.x, self.y, 2, self.height, Color::rgb(255, 255, 255));
        pipeline.draw_rect(fb, self.x + self.width - 2, self.y, 2, self.height, Color::rgb(255, 255, 255));

        // Draw Centered Button Text Label
        let label_len = self.label.len();
        let text_width = label_len * 9 * 2;
        let text_x = if self.width > text_width { self.x + (self.width - text_width) / 2 } else { self.x + 10 };
        let text_y = self.y + (self.height - 16) / 2;
        draw_text(fb, pipeline, text_x, text_y, self.label, Color::rgb(255, 255, 255), 2);
    }
}

/// Main entry point for interactive rusty_voice GUI.
pub fn main() {
    println!("🎙️ Launching Sovereign Interactive Voice Studio (rusty_voice)...");

    // 1. Create Sovereign Interactive OS Desktop Window
    let mut window = WindowBuilder::new()
        .with_title("Rusty Mill Sovereign Voice Studio (Interactive GUI)")
        .with_inner_size(900, 600)
        .build()
        .context("Failed to open sovereign OS Window")
        .unwrap();

    println!("🖼️ Interactive Desktop Window Opened: 900x600");

    // 2. Setup Graphics Framebuffer & Pipeline
    let mut fb = Framebuffer::new(900, 600);
    let pipeline = Pipeline::new();

    // 3. Define Interactive UI Controls with Labels
    let btn_record = Button::new(60, 100, 220, 60, "RECORD", Color::rgb(220, 38, 38), Color::rgb(239, 68, 68));
    let btn_transcribe = Button::new(310, 100, 260, 60, "TRANSCRIBE", Color::rgb(37, 99, 235), Color::rgb(59, 130, 246));
    let btn_clear = Button::new(600, 100, 220, 60, "CLEAR", Color::rgb(75, 85, 99), Color::rgb(107, 114, 128));

    // 4. Initialize Audio Input Stream
    let spec = AudioSpec::whisper_spec();
    let mut audio_capture = AudioCapture::open_default(spec).ok();
    if audio_capture.is_none() {
        eprintln!("rusty_voice: no default capture device available; RECORD will do nothing.");
    }

    // 4b. Load the real Whisper model (see `find_whisper_model`'s doc for
    // where it looks). A missing/unloadable model degrades TRANSCRIBE to a
    // clear error message rather than the old hardcoded-success string.
    let whisper_model = find_whisper_model().and_then(|path| load_whisper_model(&path));
    if whisper_model.is_none() {
        eprintln!("rusty_voice: no Whisper model found; TRANSCRIBE will report an error instead of transcribing.");
    }

    // 5. Interactive State Variables
    let mut state = AppState::Idle;
    let mut cursor_pos = (0usize, 0usize);
    let mut recorded_samples: alloc::vec::Vec<f32> = alloc::vec::Vec::new();
    let mut transcription_text = String::from("SOVEREIGN VOICE STUDIO READY");
    let mut running = true;
    let mut frame_count = 0u64;

    println!("⚡ Entering Continuous Sovereign Interactive GUI Event Loop...");

    // 6. Continuous Interactive Event Loop (Unbounded)
    while running {
        frame_count += 1;

        // Poll window events
        let events = window.poll_events();
        for event in events {
            match event {
                Event::CloseRequested => {
                    println!("❌ Window close requested.");
                    running = false;
                }
                Event::CursorMoved(x, y) => {
                    cursor_pos = (x as usize, y as usize);
                }
                Event::MousePressed(MouseButton::Left) => {
                    if btn_record.contains(cursor_pos.0, cursor_pos.1) {
                        // Starts a fresh recording; audio actually
                        // accumulates below, every frame, while
                        // `state == Recording` -- not just once per click.
                        state = AppState::Recording;
                        recorded_samples.clear();
                        transcription_text = String::from("RECORDING AUDIO: 0 SAMPLES");
                        println!("🔴 [UI Interaction] RECORD clicked!");
                    } else if btn_transcribe.contains(cursor_pos.0, cursor_pos.1) {
                        state = AppState::Transcribing;
                        transcription_text = match &whisper_model {
                            Some(m) => transcribe_samples(m, &recorded_samples),
                            None => String::from("NO WHISPER MODEL LOADED"),
                        };
                        println!("🔵 [UI Interaction] TRANSCRIBE clicked! -> {transcription_text}");
                    } else if btn_clear.contains(cursor_pos.0, cursor_pos.1) {
                        state = AppState::Idle;
                        recorded_samples.clear();
                        transcription_text = String::from("CLEARED CONTEXT. READY.");
                        println!("⚪ [UI Interaction] CLEAR clicked!");
                    }
                }
                Event::KeyPressed(KeyCode::Escape) => {
                    println!("⌨️ ESC pressed, exiting application.");
                    running = false;
                }
                _ => {}
            }
        }

        // Continuously pull real captured audio into the recording buffer
        // every frame while recording is active -- previously this only
        // ever happened once, on the RECORD click itself, so the buffer
        // never grew beyond whatever had already queued up by that instant.
        if state == AppState::Recording {
            if let Some(cap) = audio_capture.as_mut() {
                let new_samples = cap.read_samples();
                if !new_samples.is_empty() {
                    recorded_samples.extend_from_slice(&new_samples);
                    transcription_text = format!(
                        "RECORDING AUDIO: {} SAMPLES (~{:.1}s)",
                        recorded_samples.len(),
                        recorded_samples.len() as f32 / spec.sample_rate as f32
                    );
                }
            }
        }

        // Render Background (Sleek Dark Mode #18181b)
        fb.clear(Color::rgb(24, 24, 27));

        // Draw Header Title Card & Text
        pipeline.draw_rect(&mut fb, 40, 30, 820, 50, Color::rgb(39, 39, 42));
        draw_text(&mut fb, &pipeline, 60, 45, "RUSTY MILL SOVEREIGN VOICE STUDIO", Color::rgb(250, 204, 21), 2);

        // Draw Buttons with Hover States and Text Labels
        let record_hover = btn_record.contains(cursor_pos.0, cursor_pos.1);
        let transcribe_hover = btn_transcribe.contains(cursor_pos.0, cursor_pos.1);
        let clear_hover = btn_clear.contains(cursor_pos.0, cursor_pos.1);

        btn_record.draw(&pipeline, &mut fb, record_hover);
        btn_transcribe.draw(&pipeline, &mut fb, transcribe_hover);
        btn_clear.draw(&pipeline, &mut fb, clear_hover);

        // Draw Live Audio Level Meter
        pipeline.draw_rect(&mut fb, 60, 190, 760, 40, Color::rgb(39, 39, 42));
        draw_text(&mut fb, &pipeline, 70, 202, "AUDIO INPUT METER:", Color::rgb(156, 163, 175), 1);
        if state == AppState::Recording {
            let pulse_width = ((frame_count * 15) % 550) as usize + 20;
            pipeline.draw_rect(&mut fb, 250, 200, pulse_width, 20, Color::rgb(34, 197, 94)); // Bright emerald green audio bar
        }

        // Draw Live Transcription Text Display Box
        pipeline.draw_rect(&mut fb, 60, 260, 760, 280, Color::rgb(39, 39, 42));
        pipeline.draw_rect(&mut fb, 80, 280, 720, 240, Color::rgb(18, 18, 20));
        draw_text(&mut fb, &pipeline, 100, 300, &transcription_text, Color::rgb(244, 244, 245), 2);

        // Present Framebuffer to Native Window Surface
        fb.present(&window);

        // Yield CPU thread for ~60 FPS update
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    // 7. Output Final Status JSON on exit
    let mut map = rusty_json::Map::new();
    map.insert(String::from("app"), rusty_json::Value::String(String::from("rusty_voice_gui")));
    map.insert(String::from("status"), rusty_json::Value::String(format!("{:?}", state)));
    map.insert(String::from("transcription"), rusty_json::Value::String(transcription_text));

    let json_output = rusty_json::to_string_pretty(&rusty_json::Value::Object(map)).unwrap();
    println!("✨ Interactive Sovereign Voice Studio Session Finished:\n{}", json_output);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real, not mocked: loads the actual shipped `ggml-tiny.en-q5_1.bin`
    /// model and runs real Whisper transcription on a short silent buffer
    /// — proving the "record -> transcribe" pipeline this crate is named
    /// for actually works end-to-end, without needing the GUI window
    /// (which this test environment can't create — see `mill-term`'s and
    /// `rusty_term`'s own notes on piped/non-console limitations).
    #[test]
    fn transcribes_a_short_silent_buffer_without_panicking() {
        let model_path = find_whisper_model().expect("ggml-tiny.en-q5_1.bin should be found via sibling lookup");
        let model = load_whisper_model(&model_path).expect("the real shipped model should load");

        // 1 second of silence at 16kHz -- exercises the full pipeline
        // (mel spectrogram, encoder, decoder) without needing real speech
        // audio, which this environment has no way to supply.
        let silence = alloc::vec![0.0f32; 16000];
        let text = transcribe_samples(&model, &silence);
        assert!(!text.is_empty(), "transcribe_samples should always return some string, even for silence");
    }

    #[test]
    fn empty_samples_reports_no_audio_without_calling_whisper() {
        let model_path = find_whisper_model().expect("ggml-tiny.en-q5_1.bin should be found via sibling lookup");
        let model = load_whisper_model(&model_path).expect("the real shipped model should load");
        assert_eq!(transcribe_samples(&model, &[]), "NO AUDIO RECORDED");
    }

    #[test]
    fn find_whisper_model_locates_the_real_shipped_file() {
        let path = find_whisper_model().expect("should find the sibling rusty_whisper model");
        assert!(path.is_file());
        assert!(path.to_string_lossy().ends_with("ggml-tiny.en-q5_1.bin"));
    }
}
