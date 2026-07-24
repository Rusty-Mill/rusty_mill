//! Server configuration (mirrors h2::server::Builder).

use crate::connect::ServerSettings;

/// Configuration options for a server connection builder.
#[derive(Debug, Clone)]
pub struct Builder {
    max_concurrent_streams: u32,
    initial_stream_window_size: u32,
    max_frame_size: u32,
    max_header_list_size: usize,
    enable_push: bool,
}

impl Builder {
    /// Create a new Builder with default settings.
    pub fn new() -> Self {
        Builder {
            max_concurrent_streams: u32::MAX,
            initial_stream_window_size: 1_048_576,
            max_frame_size: 16_384,
            max_header_list_size: usize::MAX,
            enable_push: true,
        }
    }

    /// Set the peer's initial maximum number of concurrent streams.
    pub fn max_concurrent_streams(&mut self, max: u32) -> &mut Self {
        self.max_concurrent_streams = max;
        self
    }

    /// Set the initial window size for stream-level flow control.
    pub fn initial_window_size(&mut self, size: u32) -> &mut Self {
        self.initial_stream_window_size = size;
        self
    }

    /// Set the maximum frame size.
    pub fn max_frame_size(&mut self, size: u32) -> &mut Self {
        self.max_frame_size = size;
        self
    }

    /// Set the maximum header list size.
    pub fn max_header_list_size(&mut self, size: usize) -> &mut Self {
        self.max_header_list_size = size;
        self
    }

    /// Enable or disable server push.
    pub fn enable_push(&mut self, enable: bool) -> &mut Self {
        self.enable_push = enable;
        self
    }

    /// Apply these settings to a ServerSettings.
    pub fn to_settings(&self) -> ServerSettings {
        let mut settings = ServerSettings::new();
        settings.max_concurrent_streams = Some(self.max_concurrent_streams);
        settings.initial_window_size = Some(self.initial_stream_window_size);
        settings.max_frame_size = Some(self.max_frame_size);
        settings.max_header_list_size = Some(self.max_header_list_size);
        settings.enable_push = if self.enable_push { Some(1) } else { Some(0) };
        settings
    }

    /// Build the ServerSettings.
    pub fn build(self) -> ServerSettings {
        self.to_settings()
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}
