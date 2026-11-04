//! Application settings (persisted to `settings.json`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Request above response.
    #[default]
    Stacked,
    /// Request and response side by side.
    SideBySide,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxySettings {
    pub use_system: bool,
    pub use_env_vars: bool,
    pub custom_enabled: bool,
    /// Full proxy URL, e.g. `http://proxy:8080` or `socks5://h:1080`.
    pub custom_url: String,
    pub username: String,
    pub password: String,
    /// Comma-separated hosts that bypass the proxy.
    pub bypass: String,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            use_system: false,
            use_env_vars: true,
            custom_enabled: false,
            custom_url: String::new(),
            username: String::new(),
            password: String::new(),
            bypass: String::new(),
        }
    }
}

/// A client certificate used for hosts matching `host` (and `port` if set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientCert {
    pub host: String,
    pub port: Option<u16>,
    /// PEM file containing the certificate (and key, if `key_path` is empty).
    pub cert_path: String,
    pub key_path: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    // General
    /// Request timeout in milliseconds; 0 means no timeout.
    pub request_timeout_ms: u64,
    pub max_response_size_mb: u64,
    pub send_no_cache_header: bool,
    pub trim_keys_and_values: bool,
    pub ssl_verification: bool,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub working_dir: String,
    pub allow_files_outside_working_dir: bool,
    pub history_limit: usize,
    pub script_timeout_ms: u64,
    // Appearance
    pub theme: Theme,
    pub ui_zoom: f32,
    pub editor_font_size: f32,
    pub line_wrap: bool,
    pub layout: Layout,
    // Network
    pub proxy: ProxySettings,
    pub ca_cert_enabled: bool,
    pub ca_cert_path: String,
    pub client_certs: Vec<ClientCert>,
    /// Bumped when the default look changes; files without it predate 0.2.
    #[serde(default)]
    pub appearance_version: u32,
}

/// Version of the default appearance; 2 made the light theme the default.
pub const APPEARANCE_VERSION: u32 = 2;

impl Default for Settings {
    fn default() -> Self {
        Self {
            request_timeout_ms: 0,
            max_response_size_mb: 50,
            send_no_cache_header: true,
            trim_keys_and_values: false,
            ssl_verification: true,
            follow_redirects: true,
            max_redirects: 10,
            working_dir: String::new(),
            allow_files_outside_working_dir: true,
            history_limit: 1000,
            script_timeout_ms: 10_000,
            theme: Theme::Light,
            ui_zoom: 1.0,
            editor_font_size: 13.0,
            line_wrap: false,
            layout: Layout::Stacked,
            proxy: ProxySettings::default(),
            ca_cert_enabled: false,
            ca_cert_path: String::new(),
            client_certs: Vec::new(),
            appearance_version: APPEARANCE_VERSION,
        }
    }
}

impl Settings {
    /// Move settings saved by older versions onto the current defaults.
    /// Returns true when something changed.
    pub fn migrate(&mut self) -> bool {
        if self.appearance_version >= APPEARANCE_VERSION {
            return false;
        }
        // The old default was "follow the system"; only that is reset.
        if self.theme == Theme::System {
            self.theme = Theme::Light;
        }
        self.appearance_version = APPEARANCE_VERSION;
        true
    }

    pub fn response_font_size(&self) -> f32 {
        self.editor_font_size
    }

    pub fn max_response_bytes(&self) -> usize {
        (self.max_response_size_mb.max(1) as usize) * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_system_theme_moves_to_light_once() {
        let mut s: Settings = serde_json::from_str(r#"{"theme": "system"}"#).unwrap();
        assert!(s.migrate());
        assert_eq!(s.theme, Theme::Light);
        s.theme = Theme::System;
        assert!(!s.migrate());
        assert_eq!(s.theme, Theme::System);

        let mut dark: Settings = serde_json::from_str(r#"{"theme": "dark"}"#).unwrap();
        dark.migrate();
        assert_eq!(dark.theme, Theme::Dark);
        assert!(!Settings::default().migrate());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{"requestTimeoutMs": 5000, "theme": "dark"}"#).unwrap();
        assert_eq!(s.request_timeout_ms, 5000);
        assert_eq!(s.theme, Theme::Dark);
        assert!(s.ssl_verification);
        assert_eq!(s.history_limit, 1000);
    }
}
