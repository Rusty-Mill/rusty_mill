# rusty_voice

A sovereign voice-to-text desktop application, built exclusively with
Rusty Mill libraries: `rusty_gui` (window/events), `rusty_gpu`
(framebuffer/rendering), `rusty_audio` (real WASAPI microphone capture),
and `rusty-whisper` (real speech-to-text).

## What changed (2026-07-27)

Previously, `rusty_audio` was a stub (no real capture) and the TRANSCRIBE
button just set a hardcoded success string regardless of what (nothing)
had been recorded — `rusty-whisper`, the crate's own namesake dependency,
was never called at all. Now that `rusty_audio` has real WASAPI capture:

- **RECORD** continuously accumulates real captured microphone audio into
  a buffer every frame while recording is active (previously it only
  pulled samples once, at the instant of the click).
- **TRANSCRIBE** runs the accumulated buffer through real Whisper
  transcription (`rusty_whisper::transcribe::transcribe`) using the
  shipped `ggml-tiny.en-q5_1.bin` model (located via a sibling-repo lookup,
  same pattern as `mill-term`'s tool discovery — this ecosystem's repos
  are siblings under one parent directory, not one shared Cargo
  workspace), and displays the real transcribed text.
- **CLEAR** resets the recording buffer.

A missing microphone or missing/unloadable model degrades gracefully
(logged to stderr, clear on-screen message) rather than the app refusing
to start or silently pretending to work.

## Testing

The GUI itself can't be exercised in a non-interactive/piped environment
(same limitation `rusty_term`/`mill-term` document — a real window needs a
real console). The record → transcribe *pipeline* is fully testable
without the GUI, though, and `cargo test` does exactly that: loads the
real shipped model and runs real transcription on a synthetic buffer,
proving the wiring actually works end-to-end.

```
cargo test
```

## Known gaps

- Text rendering still uses a hand-rolled inline 8×8 bitmap font, not
  `rusty_font` (which is still a near-total stub as of this writing —
  swapping it in is a separate rebuild).
- No streaming/incremental transcription (`rusty_whisper::transcribe::Stream`
  exists for that) — this app does one batch `transcribe()` call when
  TRANSCRIBE is clicked, not live captions while recording.
