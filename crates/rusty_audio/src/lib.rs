#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]

//! # `rusty_audio`
//!
//! A sovereign PCM audio capture device driver for the **Rusty Mill**
//! ecosystem.
//!
//! **Windows: real, via hand-written WASAPI COM FFI** (see [`wasapi`]) —
//! opens the default microphone, captures in the device's native mix
//! format, and resamples/downmixes to the requested [`AudioSpec`].
//!
//! **Known gaps:** no playback (capture only), no Linux (ALSA) backend
//! yet despite the `rusty_libc` target dependency implying one, and no
//! 24-bit PCM native-format support (WASAPI's `GetBuffer` fails loudly
//! with a distinct error in that case rather than silently corrupting
//! audio — see [`wasapi::WasapiCapture::read_samples`]).

extern crate alloc;

use alloc::vec::Vec;

#[cfg(windows)]
pub mod wasapi;

/// Audio format specification (sample rate, channel count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioSpec {
    /// Sample rate in Hz (e.g., 16000, 44100, 48000).
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono, 2 = stereo).
    pub channels: u16,
}

impl AudioSpec {
    /// Creates a 16kHz mono audio specification suitable for Whisper.
    pub fn whisper_spec() -> Self {
        Self { sample_rate: 16000, channels: 1 }
    }
}

/// Downmixes interleaved `samples` from `channels` to mono by averaging
/// each frame. A no-op if already mono. Upmixing (fewer input channels
/// than requested) isn't implemented — a known scope cut, since every
/// real caller so far only ever downmixes to mono for Whisper.
fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples.chunks(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32).collect()
}

/// Resamples mono `samples` from `from_rate` to `to_rate` via linear
/// interpolation — not a windowed-sinc resampler, but sufficient quality
/// for feeding a speech-recognition model (Whisper itself trains on
/// 16kHz audio that was, in practice, resampled from many source rates
/// this same simple way).
fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    // `as usize` on a non-negative f64 truncates toward zero, which is
    // exactly `floor` for non-negative values -- `f64::floor` itself isn't
    // available in `core` (needs libm), hence the cast instead of a call.
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = samples[idx];
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Converts `samples` (interleaved, `from_channels` channels, at
/// `from_rate` Hz) to the mono `to_rate` Hz format Whisper (and this
/// crate's other consumers) expect.
pub fn resample_to_mono(samples: &[f32], from_rate: u32, from_channels: usize, to_rate: u32) -> Vec<f32> {
    let mono = downmix_to_mono(samples, from_channels);
    resample_linear(&mono, from_rate, to_rate)
}

/// A PCM audio capture stream, real on Windows (see [`wasapi`]).
pub struct AudioCapture {
    #[cfg(windows)]
    inner: wasapi::WasapiCapture,
    spec: AudioSpec,
}

impl AudioCapture {
    /// Opens the default system PCM microphone input stream.
    #[cfg(windows)]
    pub fn open_default(spec: AudioSpec) -> Result<Self, &'static str> {
        let inner =
            wasapi::WasapiCapture::open_default().map_err(|_| "WASAPI: failed to open the default capture device")?;
        Ok(Self { inner, spec })
    }

    /// Opens the default system PCM microphone input stream.
    ///
    /// Not yet implemented outside Windows — no ALSA (Linux) backend
    /// exists yet despite the crate depending on `rusty_libc`.
    #[cfg(not(windows))]
    pub fn open_default(_spec: AudioSpec) -> Result<Self, &'static str> {
        Err("rusty_audio: capture is only implemented for Windows (WASAPI) so far")
    }

    /// Reads whatever audio has accumulated since the last call, resampled
    /// and downmixed to this capture's requested [`AudioSpec`]. Never
    /// blocks; returns an empty `Vec` if nothing is available yet or on a
    /// transient read error.
    #[cfg(windows)]
    pub fn read_samples(&mut self) -> Vec<f32> {
        let native = self.inner.read_samples().unwrap_or_default();
        resample_to_mono(&native, self.inner.native_sample_rate(), self.inner.native_channels() as usize, self.spec.sample_rate)
    }

    /// Reads recorded samples from the stream buffer.
    #[cfg(not(windows))]
    pub fn read_samples(&mut self) -> Vec<f32> {
        Vec::new()
    }

    /// Returns the requested audio format spec (not necessarily the
    /// device's native format — see [`resample_to_mono`]).
    pub fn spec(&self) -> AudioSpec {
        self.spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_spec_initialization() {
        let spec = AudioSpec::whisper_spec();
        assert_eq!(spec.sample_rate, 16000);
        assert_eq!(spec.channels, 1);
    }

    #[test]
    fn downmix_averages_stereo_frames() {
        let stereo = [1.0, 3.0, 2.0, 4.0]; // two frames: (1,3) and (2,4)
        let mono = downmix_to_mono(&stereo, 2);
        assert_eq!(mono, alloc::vec![2.0, 3.0]);
    }

    #[test]
    fn downmix_is_a_no_op_for_mono_input() {
        let mono_in = [1.0, 2.0, 3.0];
        assert_eq!(downmix_to_mono(&mono_in, 1), alloc::vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn resample_same_rate_is_a_no_op() {
        let samples = [1.0, 2.0, 3.0];
        assert_eq!(resample_linear(&samples, 16000, 16000), alloc::vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn resample_downsamples_by_half() {
        let samples = [0.0, 1.0, 2.0, 3.0];
        let out = resample_linear(&samples, 32000, 16000);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 2.0);
    }

    #[test]
    fn resample_to_mono_downmixes_then_resamples() {
        // Stereo at 32kHz -> mono at 16kHz.
        let stereo = [0.0, 0.0, 2.0, 2.0, 4.0, 4.0, 6.0, 6.0];
        let out = resample_to_mono(&stereo, 32000, 2, 16000);
        assert_eq!(out, alloc::vec![0.0, 4.0]);
    }

    #[test]
    #[cfg(windows)]
    fn open_default_either_succeeds_or_fails_cleanly_no_panic() {
        // This sandboxed/CI environment may have no default capture
        // device at all -- the point of this test is that WASAPI failure
        // (e.g. no microphone) surfaces as a clean `Err`, not a panic or
        // crash from the hand-written COM FFI.
        if let Ok(mut capture) = AudioCapture::open_default(AudioSpec::whisper_spec()) {
            let _ = capture.read_samples();
            assert_eq!(capture.spec(), AudioSpec::whisper_spec());
        }
    }
}
