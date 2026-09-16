//! HTTP/2 settings (RFC 9113 §6.5.2).
//!
//! Each HTTP/2 connection has a set of configuration parameters (settings)
//! that define the connection's behaviour. Settings are communicated via
//! the SETTINGS frame. Each setting has a 16-bit identifier and a 32-bit
//! value.
//!
//! Settings identified by unknown identifiers MUST NOT affect the connection.

use crate::frame::settings::SettingId;
use std::fmt;

/// Local/remote settings store.
#[derive(Debug, Clone)]
pub struct ServerSettings {
    /// Size of the header table used to compress header blocks (RFC 9113 §6.5.2).
    pub header_table_size: Option<usize>,
    /// Enable/disable server push (RFC 8297 deprecated push; this should be 0).
    pub enable_push: Option<u32>,
    /// Maximum number of concurrent streams (RFC 9113 §6.5.2). A value of 0 means
    /// no additional streams may be opened.
    pub max_concurrent_streams: Option<u32>,
    /// Initial flow-control window size for new streams (RFC 9113 §6.9.1).
    pub initial_window_size: Option<u32>,
    /// Maximum frame size this implementation will send (RFC 9113 §6.5.2).
    pub max_frame_size: Option<u32>,
    /// Maximum size of header list that this implementation will accept (RFC 9113 §6.5.2).
    pub max_header_list_size: Option<usize>,
}

impl ServerSettings {
    pub fn new() -> Self {
        ServerSettings {
            header_table_size: Some(4096),
            enable_push: Some(0),
            max_concurrent_streams: None,
            initial_window_size: None,
            max_frame_size: None,
            max_header_list_size: None,
        }
    }

    /// Set a setting by ID.
    pub fn set_field(&mut self, id: SettingId, value: u32) {
        match id {
            SettingId::HeaderTableSize => self.header_table_size = Some(value as usize),
            SettingId::EnablePush => self.enable_push = Some(value),
            SettingId::MaxConcurrentStreams => self.max_concurrent_streams = Some(value),
            SettingId::InitialWindowSize => {
                self.initial_window_size = Some(value);
            }
            SettingId::MaxFrameSize => self.max_frame_size = Some(value),
            SettingId::MaxHeaderListSize => self.max_header_list_size = Some(value as usize),
            SettingId::Unknown(_) => {}
        }
    }

    /// Convert this settings object to a sequence of settings, suitable for
    /// including in a SETTINGS frame.
    pub fn encode_settings(&self) -> Vec<crate::frame::settings::Setting> {
        let mut settings = Vec::new();
        if let Some(v) = self.header_table_size {
            settings.push(crate::frame::settings::Setting {
                id: crate::frame::settings::SettingId::HeaderTableSize,
                value: v as u32,
            });
        }
        if let Some(v) = self.enable_push {
            settings.push(crate::frame::settings::Setting {
                id: crate::frame::settings::SettingId::EnablePush,
                value: v,
            });
        }
        if let Some(v) = self.max_concurrent_streams {
            settings.push(crate::frame::settings::Setting {
                id: crate::frame::settings::SettingId::MaxConcurrentStreams,
                value: v,
            });
        }
        if let Some(v) = self.initial_window_size {
            settings.push(crate::frame::settings::Setting {
                id: crate::frame::settings::SettingId::InitialWindowSize,
                value: v,
            });
        }
        if let Some(v) = self.max_frame_size {
            settings.push(crate::frame::settings::Setting {
                id: crate::frame::settings::SettingId::MaxFrameSize,
                value: v,
            });
        }
        if let Some(v) = self.max_header_list_size {
            settings.push(crate::frame::settings::Setting {
                id: crate::frame::settings::SettingId::MaxHeaderListSize,
                value: v as u32,
            });
        }
        settings
    }
}

impl fmt::Display for ServerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ServerSettings {{ header_table_size: {:?}, enable_push: {:?}, \
             max_concurrent_streams: {:?}, initial_window_size: {:?}, \
             max_frame_size: {:?}, max_header_list_size: {:?} }}",
            self.header_table_size,
            self.enable_push,
            self.max_concurrent_streams,
            self.initial_window_size,
            self.max_frame_size,
            self.max_header_list_size,
        )
    }
}
