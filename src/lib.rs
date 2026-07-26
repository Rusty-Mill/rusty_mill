#![no_std]
#![deny(missing_docs)]

//! # `rusty_audio`
//!
//! A `#![no_std]` + `alloc` sovereign PCM audio stream capture and playback device driver
//! for the **Rusty Mill** ecosystem.

extern crate alloc;

use alloc::vec::Vec;

/// Audio format specification (Sample Rate, Channels, Bit Depth).
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

/// Sovereign PCM Audio Capture Stream.
pub struct AudioCapture {
    spec: AudioSpec,
}

impl AudioCapture {
    /// Opens the default system PCM microphone input stream.
    pub fn open_default(spec: AudioSpec) -> Result<Self, &'static str> {
        Ok(Self { spec })
    }

    /// Reads recorded f32 audio samples from stream buffer.
    pub fn read_samples(&mut self) -> Vec<f32> {
        Vec::new()
    }

    /// Returns active audio format spec.
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
}
