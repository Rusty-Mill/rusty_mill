# rusty_audio

A sovereign PCM audio capture device driver for the **Rusty Mill**
ecosystem.

## Windows: real, via hand-written WASAPI COM FFI

No `windows-sys`, no `cpal` — hand-written COM vtable definitions against
`ole32.dll` (`CoInitializeEx`/`CoCreateInstance`), matching this
ecosystem's raw-FFI convention (see `rusty_win32`). Opens the default
microphone, captures in the device's native mix format (correctly
resolving `WAVEFORMATEXTENSIBLE`'s `SubFormat` GUID, not just the base
`WAVEFORMATEX.wFormatTag` — the common case for any real multi-channel
default device), and resamples/downmixes to the requested [`AudioSpec`].

Verified with real hardware in this session: opened a real 4-channel,
48kHz, IEEE-float default capture device, captured real audio, and
resampled it correctly to 16kHz mono — see `examples/wasapi_smoke.rs`.

**Two real bugs found and fixed during that verification**, not just
written and assumed correct:
1. `WAVEFORMATEXTENSIBLE` (which real hardware actually reports for any
   >2-channel device) wasn't parsed at all — only the base
   `WAVEFORMATEX.wFormatTag` was checked, so real hardware's `0xFFFE`
   ("extensible") tag was silently treated as an unsupported format.
2. Once extensible parsing was added, the extension's fields still read
   as garbage — `#[repr(C)]`'s natural alignment inserted 2 bytes of
   padding after the base `WAVEFORMATEX` that real Windows doesn't have
   (`mmreg.h` wraps these structs in `pshpack1`/`poppack`, giving
   `WAVEFORMATEX` its well-known "weird" 18-byte size, not 20). Fixed with
   `#[repr(C, packed)]` on both format structs.

## Known gaps

- **Capture only** — no playback.
- **No Linux (ALSA) backend** yet, despite the `rusty_libc` target
  dependency implying one.
- **No 24-bit (or other non-16/32-bit) native PCM support** — `read_samples`
  fails loudly with a distinct error in that case rather than silently
  corrupting audio.

## Testing

```
cargo test
cargo run --example wasapi_smoke   # captures ~2s from the real default mic
```
