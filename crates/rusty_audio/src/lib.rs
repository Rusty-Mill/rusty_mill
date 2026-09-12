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
        Self {
            sample_rate: 16000,
            channels: 1,
        }
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
    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Upper bound on how many input samples' worth of output
/// `resample_linear` will allocate per input sample. Real-world audio
/// rates span roughly 8kHz to 192kHz (a ~24x range), so capping the
/// expansion factor here is generous while still bounding the
/// allocation a valid-but-extreme `from_rate`/`to_rate` pair (e.g.
/// `from_rate = 1`, `to_rate = u32::MAX`) could otherwise force.
const MAX_RESAMPLE_EXPANSION: usize = 1024;

/// Errors from [`resample_linear`]/[`resample_to_mono`]: invalid
/// parameters, or parameters that would force an absurdly large
/// allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResampleError {
    /// `from_rate` was zero.
    ZeroSourceRate,
    /// `to_rate` was zero.
    ZeroTargetRate,
    /// `from_channels` was zero.
    ZeroChannels,
    /// The `from_rate`/`to_rate` ratio would expand the input by more
    /// than `MAX_RESAMPLE_EXPANSION`x, which for real-world audio rates
    /// only happens with a degenerate rate pair.
    ExcessiveExpansion,
}

/// Resamples mono `samples` from `from_rate` to `to_rate` via linear
/// interpolation — not a windowed-sinc resampler, but sufficient quality
/// for feeding a speech-recognition model (Whisper itself trains on
/// 16kHz audio that was, in practice, resampled from many source rates
/// this same simple way).
fn resample_linear(
    samples: &[f32],
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f32>, ResampleError> {
    if from_rate == 0 {
        return Err(ResampleError::ZeroSourceRate);
    }
    if to_rate == 0 {
        return Err(ResampleError::ZeroTargetRate);
    }
    if from_rate == to_rate || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let ratio = from_rate as f64 / to_rate as f64;
    // `as usize` on a non-negative f64 truncates toward zero, which is
    // exactly `floor` for non-negative values -- `f64::floor` itself isn't
    // available in `core` (needs libm), hence the cast instead of a call.
    let out_len_f64 = samples.len() as f64 / ratio;
    // Sanity-bound the computed output length before allocating: both
    // `from_rate` and `to_rate` are now known nonzero, but an extreme
    // (though individually valid) ratio between them can still force a
    // multi-gigabyte `Vec::with_capacity` request.
    if !out_len_f64.is_finite()
        || out_len_f64 > samples.len() as f64 * MAX_RESAMPLE_EXPANSION as f64
    {
        return Err(ResampleError::ExcessiveExpansion);
    }
    let out_len = out_len_f64 as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = samples[idx];
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    Ok(out)
}

/// Converts `samples` (interleaved, `from_channels` channels, at
/// `from_rate` Hz) to the mono `to_rate` Hz format Whisper (and this
/// crate's other consumers) expect.
pub fn resample_to_mono(
    samples: &[f32],
    from_rate: u32,
    from_channels: usize,
    to_rate: u32,
) -> Result<Vec<f32>, ResampleError> {
    if from_channels == 0 {
        return Err(ResampleError::ZeroChannels);
    }
    let mono = downmix_to_mono(samples, from_channels);
    resample_linear(&mono, from_rate, to_rate)
}

/// A PCM audio capture stream, real on Windows (see [`wasapi`]).
pub struct AudioCapture {
    #[cfg(windows)]
    inner: wasapi::WasapiCapture,
    spec: AudioSpec,
}

/// Errors from [`AudioCapture::read_samples`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCaptureError {
    /// The native WASAPI backend failed to produce samples — e.g. its
    /// own explicit "unsupported native sample format" error for 24-bit
    /// PCM (see [`wasapi::WasapiCapture::read_samples`]), or any other
    /// WASAPI failure. Propagated rather than swallowed, since either is
    /// otherwise indistinguishable from "no audio available yet".
    #[cfg(windows)]
    Native(wasapi::WasapiError),
    /// The native format couldn't be resampled to the requested
    /// [`AudioSpec`] — see [`ResampleError`].
    Resample(ResampleError),
}

#[cfg(windows)]
impl From<wasapi::WasapiError> for AudioCaptureError {
    fn from(err: wasapi::WasapiError) -> Self {
        AudioCaptureError::Native(err)
    }
}

/// The minimal capability [`read_and_resample`] needs from a native audio
/// backend — implemented for real by [`wasapi::WasapiCapture`] on
/// Windows. Exists so the read-then-resample-then-propagate-errors
/// wrapper is unit-testable (see this module's tests) without a real
/// WASAPI device.
#[cfg(windows)]
trait NativeAudioSource {
    /// The backend's own read error type.
    type Error;
    /// Reads whatever native-format samples are currently available.
    fn read_samples(&mut self) -> Result<Vec<f32>, Self::Error>;
    /// The native mix format's sample rate in Hz.
    fn native_sample_rate(&self) -> u32;
    /// The native mix format's channel count.
    fn native_channels(&self) -> u16;
}

#[cfg(windows)]
impl NativeAudioSource for wasapi::WasapiCapture {
    type Error = wasapi::WasapiError;

    fn read_samples(&mut self) -> Result<Vec<f32>, wasapi::WasapiError> {
        wasapi::WasapiCapture::read_samples(self)
    }

    fn native_sample_rate(&self) -> u32 {
        wasapi::WasapiCapture::native_sample_rate(self)
    }

    fn native_channels(&self) -> u16 {
        wasapi::WasapiCapture::native_channels(self)
    }
}

/// Reads native-format samples from `source` and resamples/downmixes
/// them to `target`, propagating both the backend's own read errors and
/// resampling-parameter errors rather than swallowing either into an
/// empty `Vec` (see [`AudioCapture::read_samples`]).
#[cfg(windows)]
fn read_and_resample<S>(source: &mut S, target: AudioSpec) -> Result<Vec<f32>, AudioCaptureError>
where
    S: NativeAudioSource,
    AudioCaptureError: From<S::Error>,
{
    let native = source.read_samples()?;
    resample_to_mono(
        &native,
        source.native_sample_rate(),
        source.native_channels() as usize,
        target.sample_rate,
    )
    .map_err(AudioCaptureError::Resample)
}

impl AudioCapture {
    /// Opens the default system PCM microphone input stream.
    #[cfg(windows)]
    pub fn open_default(spec: AudioSpec) -> Result<Self, &'static str> {
        let inner = wasapi::WasapiCapture::open_default()
            .map_err(|_| "WASAPI: failed to open the default capture device")?;
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
    /// blocks; returns an empty `Vec` if nothing is available yet, but
    /// propagates a permanent backend or resampling error (see
    /// [`AudioCaptureError`]) instead of silently returning empty samples
    /// for it.
    #[cfg(windows)]
    pub fn read_samples(&mut self) -> Result<Vec<f32>, AudioCaptureError> {
        read_and_resample(&mut self.inner, self.spec)
    }

    /// Reads recorded samples from the stream buffer.
    #[cfg(not(windows))]
    pub fn read_samples(&mut self) -> Result<Vec<f32>, AudioCaptureError> {
        Ok(Vec::new())
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
        assert_eq!(
            resample_linear(&samples, 16000, 16000).unwrap(),
            alloc::vec![1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn resample_downsamples_by_half() {
        let samples = [0.0, 1.0, 2.0, 3.0];
        let out = resample_linear(&samples, 32000, 16000).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 2.0);
    }

    #[test]
    fn resample_to_mono_downmixes_then_resamples() {
        // Stereo at 32kHz -> mono at 16kHz.
        let stereo = [0.0, 0.0, 2.0, 2.0, 4.0, 4.0, 6.0, 6.0];
        let out = resample_to_mono(&stereo, 32000, 2, 16000).unwrap();
        assert_eq!(out, alloc::vec![0.0, 4.0]);
    }

    #[test]
    fn resample_linear_rejects_zero_source_rate_instead_of_hanging() {
        let samples = [1.0, 2.0, 3.0];
        assert_eq!(
            resample_linear(&samples, 0, 16000),
            Err(ResampleError::ZeroSourceRate)
        );
    }

    #[test]
    fn resample_linear_rejects_zero_target_rate() {
        let samples = [1.0, 2.0, 3.0];
        assert_eq!(
            resample_linear(&samples, 16000, 0),
            Err(ResampleError::ZeroTargetRate)
        );
    }

    #[test]
    fn resample_to_mono_rejects_zero_source_rate() {
        let samples = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(
            resample_to_mono(&samples, 0, 2, 16000),
            Err(ResampleError::ZeroSourceRate)
        );
    }

    #[test]
    fn resample_to_mono_rejects_zero_target_rate() {
        let samples = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(
            resample_to_mono(&samples, 16000, 2, 0),
            Err(ResampleError::ZeroTargetRate)
        );
    }

    #[test]
    fn resample_to_mono_rejects_zero_channels_instead_of_treating_as_mono() {
        let samples = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(
            resample_to_mono(&samples, 16000, 0, 16000),
            Err(ResampleError::ZeroChannels)
        );
    }

    #[test]
    fn resample_linear_rejects_extreme_rate_ratio_instead_of_huge_allocation() {
        // Both rates are individually valid (nonzero), but the ratio
        // between them would ask for a multi-billion-sample allocation
        // from a 3-sample input -- must be rejected, not attempted.
        let samples = [1.0, 2.0, 3.0];
        assert_eq!(
            resample_linear(&samples, 1, u32::MAX),
            Err(ResampleError::ExcessiveExpansion)
        );
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

    /// A [`NativeAudioSource`] test double that always fails the same way
    /// [`wasapi::WasapiCapture::read_samples`] does for an unsupported
    /// native sample format (e.g. 24-bit PCM).
    #[cfg(windows)]
    struct UnsupportedFormatSource;

    #[cfg(windows)]
    impl NativeAudioSource for UnsupportedFormatSource {
        type Error = wasapi::WasapiError;

        fn read_samples(&mut self) -> Result<Vec<f32>, wasapi::WasapiError> {
            Err(wasapi::WasapiError(-1))
        }

        fn native_sample_rate(&self) -> u32 {
            48000
        }

        fn native_channels(&self) -> u16 {
            2
        }
    }

    #[test]
    #[cfg(windows)]
    fn read_samples_propagates_unsupported_format_error_instead_of_swallowing_it() {
        let mut source = UnsupportedFormatSource;
        let result = read_and_resample(&mut source, AudioSpec::whisper_spec());
        assert_eq!(
            result,
            Err(AudioCaptureError::Native(wasapi::WasapiError(-1)))
        );
    }
}
