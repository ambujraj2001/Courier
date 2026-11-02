//! Platform differences: fonts, opening files and URLs, and how keyboard
//! shortcuts are written (⌘ on macOS, Ctrl elsewhere).

use std::ffi::OsStr;

pub const IS_MAC: bool = cfg!(target_os = "macos");

/// Open a URL or path with the desktop's default handler.
pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<()> {
    let target = target.as_ref();
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        // Not `cmd /C start`: cmd would treat `&` in URLs as a command separator.
        if target.to_string_lossy().contains("://") {
            let mut c = std::process::Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        } else {
            std::process::Command::new("explorer")
        }
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(target).spawn().map(|_| ())
}

/// What the system file manager is called, for "Show in …" buttons.
pub fn file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(target_os = "windows") {
        "Explorer"
    } else {
        "Files"
    }
}

/// Font files to try, in order: (proportional, monospace, symbol fallbacks).
/// The first proportional and monospace file that exists is used; egui's
/// bundled fonts remain as the final fallback.
pub fn font_candidates() -> (&'static [&'static str], &'static [&'static str], &'static [&'static str]) {
    if cfg!(target_os = "macos") {
        (
            &["/System/Library/Fonts/SFNS.ttf"],
            &["/System/Library/Fonts/SFNSMono.ttf"],
            &["/System/Library/Fonts/Apple Symbols.ttf"],
        )
    } else if cfg!(target_os = "windows") {
        (
            &["C:\\Windows\\Fonts\\segoeui.ttf"],
            &["C:\\Windows\\Fonts\\consola.ttf"],
            &["C:\\Windows\\Fonts\\seguisym.ttf"],
        )
    } else {
        (
            &[
                "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
                "/usr/share/fonts/truetype/ubuntu/Ubuntu[wdth,wght].ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ],
            &[
                "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
                "/usr/share/fonts/truetype/ubuntu/UbuntuMono[wght].ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
            ],
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            ],
        )
    }
}

fn modifier_name(c: char) -> Option<&'static str> {
    match c {
        '⌘' | '⌃' => Some("Ctrl"),
        '⌥' => Some("Alt"),
        '⇧' => Some("Shift"),
        _ => None,
    }
}

/// One key of a shortcut, e.g. "⌘" → "Ctrl" off macOS.
pub fn key_token(token: &str) -> String {
    if IS_MAC {
        return token.to_string();
    }
    let mut chars = token.chars();
    match (chars.next().and_then(modifier_name), chars.next()) {
        (Some(name), None) => name.to_string(),
        _ => token.to_string(),
    }
}

/// Rewrite macOS shortcut notation for the current platform:
/// "Save (⌘S)" stays as is on macOS and becomes "Save (Ctrl+S)" elsewhere;
/// "⌥⌘C" becomes "Ctrl+Alt+C".
pub fn keys(text: &str) -> String {
    if IS_MAC {
        return text.to_string();
    }
    translate(text)
}

fn translate(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if modifier_name(c).is_none() {
            out.push(c);
            continue;
        }
        let mut mods = vec![c];
        while let Some(&n) = chars.peek() {
            if modifier_name(n).is_some() {
                mods.push(n);
                chars.next();
            } else {
                break;
            }
        }
        // Conventional order, without duplicates (⌘ and ⌃ are both Ctrl).
        let mut names: Vec<&str> = Vec::new();
        for want in ["Ctrl", "Alt", "Shift"] {
            if mods.iter().any(|m| modifier_name(*m) == Some(want)) {
                names.push(want);
            }
        }
        let mut key = String::new();
        while let Some(&n) = chars.peek() {
            if n.is_whitespace() || n == ')' || n == ',' || n == '·' {
                break;
            }
            key.push(n);
            chars.next();
        }
        out.push_str(&names.join("+"));
        if !key.is_empty() {
            out.push('+');
            out.push_str(&key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_mac_shortcuts() {
        assert_eq!(translate("Save (⌘S) · right-click"), "Save (Ctrl+S) · right-click");
        assert_eq!(translate("⌥⌘C"), "Ctrl+Alt+C");
        assert_eq!(translate("Runner (⇧⌘R)"), "Runner (Ctrl+Shift+R)");
        assert_eq!(translate("Keyboard shortcuts (⌘/)"), "Keyboard shortcuts (Ctrl+/)");
        assert_eq!(translate("no shortcut"), "no shortcut");
    }

    #[test]
    fn key_tokens() {
        if !IS_MAC {
            assert_eq!(key_token("⌘"), "Ctrl");
            assert_eq!(key_token("⇧"), "Shift");
        }
        assert_eq!(key_token("Enter"), "Enter");
    }
}
