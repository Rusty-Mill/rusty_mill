//! Real WASAPI PCM capture: hand-written COM vtable FFI against
//! `ole32.dll` (`CoInitializeEx`/`CoCreateInstance`) — no `windows-sys`,
//! matching this ecosystem's raw-FFI convention (see `rusty_win32`).
//!
//! Only capture (microphone input) is implemented; playback and Linux
//! (ALSA) are known, undocumented-no-longer gaps — see the crate's
//! top-level doc comment.

use core::ffi::c_void;
use core::ptr;

use alloc::vec::Vec;

type Hresult = i32;
type RawIUnknown = *mut c_void;

#[derive(Clone, Copy)]
#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xBCDE0395,
    data2: 0xE52F,
    data3: 0x467C,
    data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
};
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xA95664D2,
    data2: 0x9614,
    data3: 0x4F35,
    data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
};
const IID_IAUDIO_CLIENT: Guid = Guid {
    data1: 0x1CB9AD4C,
    data2: 0xDBFA,
    data3: 0x4C32,
    data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
};
const IID_IAUDIO_CAPTURE_CLIENT: Guid = Guid {
    data1: 0xC8ADBD64,
    data2: 0xE71E,
    data3: 0x48A0,
    data4: [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17],
};

const CLSCTX_ALL: u32 = 0x17;
const E_CAPTURE: u32 = 1; // EDataFlow::eCapture
const E_CONSOLE: u32 = 0; // ERole::eConsole
const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
const COINIT_MULTITHREADED: u32 = 0x0;
const RPC_E_CHANGED_MODE: Hresult = 0x80010106u32 as i32;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
/// `GetMixFormat`'s typical answer for any device with more than 2
/// channels (true of most default endpoints on real hardware, and of the
/// virtual multi-channel device this exact code hit in testing): the base
/// `WAVEFORMATEX.wFormatTag` is `WAVE_FORMAT_EXTENSIBLE`, and the *real*
/// sample format lives in the `WAVEFORMATEXTENSIBLE` extension appended
/// after it (`SubFormat`, a GUID) — not in `wFormatTag` directly.
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: Guid = Guid {
    data1: 0x0000_0003,
    data2: 0x0000,
    data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
};

impl PartialEq for Guid {
    fn eq(&self, other: &Self) -> bool {
        self.data1 == other.data1
            && self.data2 == other.data2
            && self.data3 == other.data3
            && self.data4 == other.data4
    }
}

// `WAVEFORMATEX` is documented (and empirically verified: see this
// module's fix history) to be exactly 18 bytes on real Windows -- 2 bytes
// short of the 20-byte size `#[repr(C)]`'s natural alignment would
// otherwise round up to, because `mmreg.h` wraps it in
// `pshpack1.h`/`poppack.h` (byte packing) precisely so `cbSize` extra
// bytes (e.g. a `WAVEFORMATEXTENSIBLE`'s extension) follow with zero gap.
// Rust's default `#[repr(C)]` doesn't know that and inserts the "natural"
// padding, which silently shifted every field read out of the appended
// extension by 2 bytes until this was caught via a real capture device
// reporting a `WAVEFORMATEXTENSIBLE` mix format in testing.
#[repr(C, packed)]
struct WaveFormatEx {
    format_tag: u16,
    channels: u16,
    samples_per_sec: u32,
    avg_bytes_per_sec: u32,
    block_align: u16,
    bits_per_sample: u16,
    cb_size: u16,
}

/// `WAVEFORMATEXTENSIBLE`: the base `WAVEFORMATEX` above, followed by a
/// 2-byte union (`wValidBitsPerSample`, unused here), a channel mask, and
/// the `SubFormat` GUID that carries the real sample format for anything
/// `GetMixFormat` reports as [`WAVE_FORMAT_EXTENSIBLE`]. Also packed, for
/// the same reason as `WaveFormatEx` above — real `sizeof` is 40 bytes.
#[repr(C, packed)]
struct WaveFormatExtensible {
    format: WaveFormatEx,
    valid_bits_per_sample: u16,
    channel_mask: u32,
    sub_format: Guid,
}

// Every vtable below mirrors the real COM interface layout (Windows SDK
// `mmdeviceapi.h`/`audioclient.h`), `IUnknown`'s three slots first.
#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(RawIUnknown, *const Guid, *mut RawIUnknown) -> Hresult,
    add_ref: unsafe extern "system" fn(RawIUnknown) -> u32,
    release: unsafe extern "system" fn(RawIUnknown) -> u32,
}

#[repr(C)]
struct IMmDeviceEnumeratorVtbl {
    base: IUnknownVtbl,
    enum_audio_endpoints:
        unsafe extern "system" fn(RawIUnknown, u32, u32, *mut RawIUnknown) -> Hresult,
    get_default_audio_endpoint:
        unsafe extern "system" fn(RawIUnknown, u32, u32, *mut RawIUnknown) -> Hresult,
    get_device: unsafe extern "system" fn(RawIUnknown, *const u16, *mut RawIUnknown) -> Hresult,
    register_endpoint_notification_callback:
        unsafe extern "system" fn(RawIUnknown, RawIUnknown) -> Hresult,
    unregister_endpoint_notification_callback:
        unsafe extern "system" fn(RawIUnknown, RawIUnknown) -> Hresult,
}

#[repr(C)]
struct IMmDeviceVtbl {
    base: IUnknownVtbl,
    activate: unsafe extern "system" fn(
        RawIUnknown,
        *const Guid,
        u32,
        *const c_void,
        *mut RawIUnknown,
    ) -> Hresult,
    open_property_store: unsafe extern "system" fn(RawIUnknown, u32, *mut RawIUnknown) -> Hresult,
    get_id: unsafe extern "system" fn(RawIUnknown, *mut *mut u16) -> Hresult,
    get_state: unsafe extern "system" fn(RawIUnknown, *mut u32) -> Hresult,
}

#[repr(C)]
struct IAudioClientVtbl {
    base: IUnknownVtbl,
    initialize: unsafe extern "system" fn(
        RawIUnknown,
        u32,
        u32,
        i64,
        i64,
        *const WaveFormatEx,
        *const Guid,
    ) -> Hresult,
    get_buffer_size: unsafe extern "system" fn(RawIUnknown, *mut u32) -> Hresult,
    get_stream_latency: unsafe extern "system" fn(RawIUnknown, *mut i64) -> Hresult,
    get_current_padding: unsafe extern "system" fn(RawIUnknown, *mut u32) -> Hresult,
    is_format_supported: unsafe extern "system" fn(
        RawIUnknown,
        u32,
        *const WaveFormatEx,
        *mut *mut WaveFormatEx,
    ) -> Hresult,
    get_mix_format: unsafe extern "system" fn(RawIUnknown, *mut *mut WaveFormatEx) -> Hresult,
    get_device_period: unsafe extern "system" fn(RawIUnknown, *mut i64, *mut i64) -> Hresult,
    start: unsafe extern "system" fn(RawIUnknown) -> Hresult,
    stop: unsafe extern "system" fn(RawIUnknown) -> Hresult,
    reset: unsafe extern "system" fn(RawIUnknown) -> Hresult,
    set_event_handle: unsafe extern "system" fn(RawIUnknown, *mut c_void) -> Hresult,
    get_service: unsafe extern "system" fn(RawIUnknown, *const Guid, *mut RawIUnknown) -> Hresult,
}

#[repr(C)]
struct IAudioCaptureClientVtbl {
    base: IUnknownVtbl,
    get_buffer: unsafe extern "system" fn(
        RawIUnknown,
        *mut *mut u8,
        *mut u32,
        *mut u32,
        *mut u64,
        *mut u64,
    ) -> Hresult,
    release_buffer: unsafe extern "system" fn(RawIUnknown, u32) -> Hresult,
    get_next_packet_size: unsafe extern "system" fn(RawIUnknown, *mut u32) -> Hresult,
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const c_void, co_init: u32) -> Hresult;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const Guid,
        unk_outer: RawIUnknown,
        cls_context: u32,
        riid: *const Guid,
        out: *mut RawIUnknown,
    ) -> Hresult;
    fn CoTaskMemFree(ptr: *mut c_void);
}

/// Errors opening or reading from a WASAPI capture stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasapiError(pub i32);

fn check(hr: Hresult) -> Result<(), WasapiError> {
    if hr < 0 { Err(WasapiError(hr)) } else { Ok(()) }
}

macro_rules! vcall {
    ($obj:expr, $vtbl:ty, $method:ident $(, $arg:expr)*) => {{
        let obj: RawIUnknown = $obj;
        let vtbl = *(obj as *const *const $vtbl);
        ((*vtbl).$method)(obj $(, $arg)*)
    }};
}

/// A live WASAPI capture session on the default microphone.
pub struct WasapiCapture {
    device_enumerator: RawIUnknown,
    device: RawIUnknown,
    audio_client: RawIUnknown,
    capture_client: RawIUnknown,
    mix_format: *mut WaveFormatEx,
    /// Whether this instance called `CoInitializeEx` and therefore owns
    /// pairing it with `CoUninitialize` (skipped if COM was already
    /// initialized on this thread with a different concurrency model —
    /// `RPC_E_CHANGED_MODE` — since we didn't take out that reference).
    owns_com_init: bool,
}

// SAFETY: WASAPI's COM objects here are only ever touched through this
// struct's own methods, which take `&mut self`, so there is no concurrent
// access — the underlying COM apartment threading model is respected by
// never sharing these pointers across threads without synchronization.
unsafe impl Send for WasapiCapture {}

impl WasapiCapture {
    /// Opens the default system microphone for capture, in whatever
    /// native mix format (sample rate/channel count) the device reports —
    /// see [`WasapiCapture::native_channels`]/[`WasapiCapture::native_sample_rate`].
    pub fn open_default() -> Result<Self, WasapiError> {
        unsafe {
            let init_hr = CoInitializeEx(ptr::null(), COINIT_MULTITHREADED);
            let owns_com_init = init_hr != RPC_E_CHANGED_MODE;
            if init_hr < 0 && init_hr != RPC_E_CHANGED_MODE {
                return Err(WasapiError(init_hr));
            }

            let mut enumerator: RawIUnknown = ptr::null_mut();
            check(CoCreateInstance(
                &CLSID_MM_DEVICE_ENUMERATOR,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IMM_DEVICE_ENUMERATOR,
                &mut enumerator,
            ))
            .inspect_err(|_| {
                if owns_com_init {
                    CoUninitialize();
                }
            })?;

            let mut device: RawIUnknown = ptr::null_mut();
            let hr = vcall!(
                enumerator,
                IMmDeviceEnumeratorVtbl,
                get_default_audio_endpoint,
                E_CAPTURE,
                E_CONSOLE,
                &mut device
            );
            if hr < 0 {
                vcall!(enumerator, IUnknownVtbl, release);
                if owns_com_init {
                    CoUninitialize();
                }
                return Err(WasapiError(hr));
            }

            let mut audio_client: RawIUnknown = ptr::null_mut();
            let hr = vcall!(
                device,
                IMmDeviceVtbl,
                activate,
                &IID_IAUDIO_CLIENT,
                CLSCTX_ALL,
                ptr::null(),
                &mut audio_client
            );
            if hr < 0 {
                vcall!(device, IUnknownVtbl, release);
                vcall!(enumerator, IUnknownVtbl, release);
                if owns_com_init {
                    CoUninitialize();
                }
                return Err(WasapiError(hr));
            }

            let mut mix_format: *mut WaveFormatEx = ptr::null_mut();
            let hr = vcall!(
                audio_client,
                IAudioClientVtbl,
                get_mix_format,
                &mut mix_format
            );
            if hr < 0 {
                vcall!(audio_client, IUnknownVtbl, release);
                vcall!(device, IUnknownVtbl, release);
                vcall!(enumerator, IUnknownVtbl, release);
                if owns_com_init {
                    CoUninitialize();
                }
                return Err(WasapiError(hr));
            }

            // 300ms shared-mode buffer -- generous enough that a caller
            // polling every ~100ms via `read_samples` never overruns it.
            const BUFFER_DURATION_100NS: i64 = 3_000_000;
            let hr = vcall!(
                audio_client,
                IAudioClientVtbl,
                initialize,
                AUDCLNT_SHAREMODE_SHARED,
                0,
                BUFFER_DURATION_100NS,
                0,
                mix_format,
                ptr::null()
            );
            if hr < 0 {
                CoTaskMemFree(mix_format as *mut c_void);
                vcall!(audio_client, IUnknownVtbl, release);
                vcall!(device, IUnknownVtbl, release);
                vcall!(enumerator, IUnknownVtbl, release);
                if owns_com_init {
                    CoUninitialize();
                }
                return Err(WasapiError(hr));
            }

            let mut capture_client: RawIUnknown = ptr::null_mut();
            let hr = vcall!(
                audio_client,
                IAudioClientVtbl,
                get_service,
                &IID_IAUDIO_CAPTURE_CLIENT,
                &mut capture_client
            );
            if hr < 0 {
                CoTaskMemFree(mix_format as *mut c_void);
                vcall!(audio_client, IUnknownVtbl, release);
                vcall!(device, IUnknownVtbl, release);
                vcall!(enumerator, IUnknownVtbl, release);
                if owns_com_init {
                    CoUninitialize();
                }
                return Err(WasapiError(hr));
            }

            let hr = vcall!(audio_client, IAudioClientVtbl, start);
            if hr < 0 {
                vcall!(capture_client, IUnknownVtbl, release);
                CoTaskMemFree(mix_format as *mut c_void);
                vcall!(audio_client, IUnknownVtbl, release);
                vcall!(device, IUnknownVtbl, release);
                vcall!(enumerator, IUnknownVtbl, release);
                if owns_com_init {
                    CoUninitialize();
                }
                return Err(WasapiError(hr));
            }

            Ok(WasapiCapture {
                device_enumerator: enumerator,
                device,
                audio_client,
                capture_client,
                mix_format,
                owns_com_init,
            })
        }
    }

    /// The native mix format's channel count (mono/stereo/etc.).
    pub fn native_channels(&self) -> u16 {
        unsafe { (*self.mix_format).channels }
    }

    /// The native mix format's sample rate in Hz.
    pub fn native_sample_rate(&self) -> u32 {
        unsafe { (*self.mix_format).samples_per_sec }
    }

    /// The native mix format's bits-per-sample (diagnostic; `read_samples`
    /// only actually supports 32-bit float and 16-bit PCM natively).
    pub fn native_bits_per_sample(&self) -> u16 {
        unsafe { (*self.mix_format).bits_per_sample }
    }

    /// The raw `WAVEFORMATEX.wFormatTag` (diagnostic — `1` = PCM, `3` =
    /// IEEE float, `0xFFFE` = extensible, see [`Self::mix_format_is_ieee_float`]
    /// for resolving the real format in the extensible case).
    pub fn native_format_tag(&self) -> u16 {
        unsafe { (*self.mix_format).format_tag }
    }

    /// The raw `WAVEFORMATEX.cbSize` extension byte count (diagnostic).
    pub fn native_cb_size(&self) -> u16 {
        unsafe { (*self.mix_format).cb_size }
    }

    /// The raw `WAVEFORMATEXTENSIBLE.SubFormat` GUID bytes, if `cbSize`
    /// is large enough to contain one (diagnostic).
    pub fn native_sub_format_bytes(&self) -> Option<(u32, u16, u16, [u8; 8])> {
        unsafe {
            if (*self.mix_format).cb_size as usize >= 22 {
                let ext = self.mix_format as *const WaveFormatExtensible;
                let g: Guid = ptr::addr_of!((*ext).sub_format).read_unaligned();
                Some((g.data1, g.data2, g.data3, g.data4))
            } else {
                None
            }
        }
    }

    /// Whether the native mix format's actual samples are IEEE float —
    /// resolving through the `WAVEFORMATEXTENSIBLE` extension's
    /// `SubFormat` GUID when `wFormatTag` is [`WAVE_FORMAT_EXTENSIBLE`]
    /// (the common case for any device `GetMixFormat` reports with more
    /// than 2 channels), not just the base `WAVEFORMATEX.wFormatTag`.
    fn mix_format_is_ieee_float(&self) -> bool {
        unsafe {
            let format_tag = (*self.mix_format).format_tag;
            if format_tag == WAVE_FORMAT_IEEE_FLOAT {
                true
            } else if format_tag == WAVE_FORMAT_EXTENSIBLE
                && (*self.mix_format).cb_size as usize >= 22
            {
                let ext = self.mix_format as *const WaveFormatExtensible;
                let sub_format: Guid = ptr::addr_of!((*ext).sub_format).read_unaligned();
                sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            } else {
                false
            }
        }
    }

    /// Reads whatever audio has accumulated since the last call, as
    /// interleaved `f32` samples in the device's native format/channel
    /// count (see [`Self::native_sample_rate`]/[`Self::native_channels`]) —
    /// resampling/downmixing to a target spec is the caller's job (see
    /// [`crate::resample_to_mono_16k`]). Never blocks; returns an empty
    /// `Vec` if nothing is available yet.
    pub fn read_samples(&mut self) -> Result<Vec<f32>, WasapiError> {
        let mut out = Vec::new();
        unsafe {
            loop {
                let mut packet_frames: u32 = 0;
                check(vcall!(
                    self.capture_client,
                    IAudioCaptureClientVtbl,
                    get_next_packet_size,
                    &mut packet_frames
                ))?;
                if packet_frames == 0 {
                    break;
                }

                let mut data: *mut u8 = ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                check(vcall!(
                    self.capture_client,
                    IAudioCaptureClientVtbl,
                    get_buffer,
                    &mut data,
                    &mut frames,
                    &mut flags,
                    ptr::null_mut(),
                    ptr::null_mut()
                ))?;

                const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
                let channels = self.native_channels() as usize;
                let is_float = self.mix_format_is_ieee_float();
                let bits = (*self.mix_format).bits_per_sample;

                if flags & AUDCLNT_BUFFERFLAGS_SILENT != 0 || data.is_null() {
                    out.extend(core::iter::repeat_n(0.0f32, frames as usize * channels));
                } else if is_float && bits == 32 {
                    let samples =
                        core::slice::from_raw_parts(data as *const f32, frames as usize * channels);
                    out.extend_from_slice(samples);
                } else if bits == 16 {
                    let samples =
                        core::slice::from_raw_parts(data as *const i16, frames as usize * channels);
                    out.extend(samples.iter().map(|&s| s as f32 / i16::MAX as f32));
                } else {
                    // Unsupported native sample format (e.g. 24-bit PCM) --
                    // a known gap rather than silently corrupting audio.
                    vcall!(
                        self.capture_client,
                        IAudioCaptureClientVtbl,
                        release_buffer,
                        frames
                    );
                    return Err(WasapiError(-1));
                }

                check(vcall!(
                    self.capture_client,
                    IAudioCaptureClientVtbl,
                    release_buffer,
                    frames
                ))?;
            }
        }
        Ok(out)
    }
}

impl Drop for WasapiCapture {
    fn drop(&mut self) {
        unsafe {
            let _ = vcall!(self.audio_client, IAudioClientVtbl, stop);
            vcall!(self.capture_client, IUnknownVtbl, release);
            CoTaskMemFree(self.mix_format as *mut c_void);
            vcall!(self.audio_client, IUnknownVtbl, release);
            vcall!(self.device, IUnknownVtbl, release);
            vcall!(self.device_enumerator, IUnknownVtbl, release);
            if self.owns_com_init {
                CoUninitialize();
            }
        }
    }
}
