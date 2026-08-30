//! Manual smoke test: opens the default microphone and captures ~2
//! seconds of real audio, resampled to 16kHz mono (Whisper's expected
//! format). Run with `cargo run --example wasapi_smoke`.

fn main() {
    let spec = rusty_audio::AudioSpec::whisper_spec();
    match rusty_audio::AudioCapture::open_default(spec) {
        Ok(mut capture) => {
            println!("Opened default capture device at {:?}", capture.spec());
            let mut total = 0usize;
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                total += capture.read_samples().len();
            }
            println!(
                "Captured {total} samples (~{:.2}s at 16kHz)",
                total as f32 / 16000.0
            );
        }
        Err(e) => {
            println!("Failed to open the default capture device: {e:?} (no microphone available?)")
        }
    }
}
