//! Lightweight syntax highlighting for JSON, JavaScript, XML/HTML and
//! `{{variable}}` references, producing egui layout jobs.

use crate::vars::VarContext;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Plain,
    Json,
    Js,
    Xml,
}

struct Palette {
    text: Color32,
    string: Color32,
    number: Color32,
    keyword: Color32,
    comment: Color32,
    punct: Color32,
    tag: Color32,
    var_ok: Color32,
    var_bad: Color32,
    search: Color32,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            text: Color32::from_rgb(0xd8, 0xdc, 0xe2),
            string: Color32::from_rgb(0x9c, 0xd6, 0x8a),
            number: Color32::from_rgb(0xf0, 0xa8, 0x6a),
            keyword: Color32::from_rgb(0xc6, 0x9c, 0xff),
            comment: Color32::from_rgb(0x7d, 0x86, 0x92),
            punct: Color32::from_rgb(0xa8, 0xb0, 0xbb),
            tag: Color32::from_rgb(0x6c, 0xb6, 0xff),
            var_ok: Color32::from_rgb(0x2e, 0xc4, 0xb6),
            var_bad: Color32::from_rgb(0xff, 0x6b, 0x6b),
            search: Color32::from_rgba_unmultiplied(0xff, 0xd0, 0x40, 0x70),
        }
    } else {
        Palette {
            text: Color32::from_rgb(0x1f, 0x23, 0x28),
            string: Color32::from_rgb(0x1a, 0x7f, 0x37),
            number: Color32::from_rgb(0xb3, 0x5c, 0x00),
            keyword: Color32::from_rgb(0x7a, 0x3d, 0xb8),
            comment: Color32::from_rgb(0x6e, 0x77, 0x81),
            punct: Color32::from_rgb(0x55, 0x5f, 0x6a),
            tag: Color32::from_rgb(0x1f, 0x5f, 0xbf),
            var_ok: Color32::from_rgb(0x0f, 0x8b, 0x8d),
            var_bad: Color32::from_rgb(0xc0, 0x2b, 0x2b),
            search: Color32::from_rgba_unmultiplied(0xff, 0xc8, 0x00, 0x80),
        }
    }
}

const JS_KEYWORDS: &[&str] = &[
    "var",
    "let",
    "const",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "break",
    "continue",
    "new",
    "typeof",
    "instanceof",
    "in",
    "of",
    "try",
    "catch",
    "finally",
    "throw",
    "switch",
    "case",
    "default",
    "this",
    "true",
    "false",
    "null",
    "undefined",
    "async",
    "await",
    "class",
    "extends",
    "delete",
    "void",
];

/// Token kind for a byte range.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Str,
    Num,
    Kw,
    Comment,
    Punct,
    Tag,
}

fn tokenize(text: &str, lang: Lang) -> Vec<(usize, usize, Kind)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<(usize, usize, Kind)>, s: usize, e: usize, k: Kind| {
        if e > s {
            out.push((s, e, k));
        }
    };
    match lang {
        Lang::Plain => push(&mut out, 0, b.len(), Kind::Text),
        Lang::Xml => {
            while i < b.len() {
                if text[i..].starts_with("<!--") {
                    let end = text[i..].find("-->").map(|p| i + p + 3).unwrap_or(b.len());
                    push(&mut out, i, end, Kind::Comment);
                    i = end;
                } else if b[i] == b'<' {
                    let end = text[i..].find('>').map(|p| i + p + 1).unwrap_or(b.len());
                    // Highlight attribute values inside the tag.
                    let tag = &text[i..end];
                    let mut j = 0;
                    let tb = tag.as_bytes();
                    let mut seg = 0;
                    while j < tb.len() {
                        if tb[j] == b'"' || tb[j] == b'\'' {
                            let q = tb[j];
                            push(&mut out, i + seg, i + j, Kind::Tag);
                            let close = tag[j + 1..].find(q as char).map(|p| j + 1 + p + 1).unwrap_or(tb.len());
                            push(&mut out, i + j, i + close, Kind::Str);
                            j = close;
                            seg = close;
                        } else {
                            j += 1;
                        }
                    }
                    push(&mut out, i + seg, end, Kind::Tag);
                    i = end;
                } else {
                    let end = text[i..].find('<').map(|p| i + p).unwrap_or(b.len());
                    push(&mut out, i, end, Kind::Text);
                    i = end;
                }
            }
        }
        Lang::Json | Lang::Js => {
            while i < b.len() {
                let c = b[i];
                if c == b'"' || (lang == Lang::Js && (c == b'\'' || c == b'`')) {
                    let mut j = i + 1;
                    while j < b.len() && b[j] != c {
                        if b[j] == b'\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                    let end = (j + 1).min(b.len());
                    push(&mut out, i, end, Kind::Str);
                    i = end;
                } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
                    let end = text[i..].find('\n').map(|p| i + p).unwrap_or(b.len());
                    push(&mut out, i, end, Kind::Comment);
                    i = end;
                } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
                    let end = text[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(b.len());
                    push(&mut out, i, end, Kind::Comment);
                    i = end;
                } else if c.is_ascii_digit() || (c == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                    let mut j = i + 1;
                    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'_') {
                        j += 1;
                    }
                    push(&mut out, i, j, Kind::Num);
                    i = j;
                } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
                    let mut j = i + 1;
                    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'$') {
                        j += 1;
                    }
                    let word = &text[i..j];
                    let kw = if lang == Lang::Json {
                        matches!(word, "true" | "false" | "null")
                    } else {
                        JS_KEYWORDS.contains(&word)
                    };
                    push(&mut out, i, j, if kw { Kind::Kw } else { Kind::Text });
                    i = j;
                } else if b"{}[](),:;.=+-*<>!&|?".contains(&c) {
                    push(&mut out, i, i + 1, Kind::Punct);
                    i += 1;
                } else {
                    // Copy a whole UTF-8 character.
                    let len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
                    push(&mut out, i, i + len, Kind::Text);
                    i += len;
                }
            }
        }
    }
    out
}

/// Build a layout job with syntax colours, `{{variable}}` highlighting
/// (green when resolvable, red when not) and search-match backgrounds.
pub fn layout(
    text: &str,
    lang: Lang,
    font: FontId,
    dark: bool,
    vars: Option<&VarContext>,
    search: &str,
    wrap_width: f32,
) -> LayoutJob {
    let p = palette(dark);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let var_ranges: Vec<(usize, usize, bool)> = match vars {
        Some(v) => {
            VarContext::references(text).into_iter().map(|(s, e, name)| (s, e, v.lookup(&name).is_some())).collect()
        }
        None => Vec::new(),
    };
    let search_ranges: Vec<(usize, usize)> = if search.is_empty() {
        Vec::new()
    } else {
        let lower = text.to_lowercase();
        let needle = search.to_lowercase();
        // Lower-casing can change byte lengths for some scripts; only use the
        // matches when offsets line up.
        if lower.len() == text.len() {
            lower.match_indices(&needle).map(|(i, m)| (i, i + m.len())).collect()
        } else {
            Vec::new()
        }
    };

    // Split tokens further at variable and search boundaries.
    let mut cuts: Vec<usize> = var_ranges.iter().flat_map(|(s, e, _)| [*s, *e]).collect();
    cuts.extend(search_ranges.iter().flat_map(|(s, e)| [*s, *e]));
    cuts.sort_unstable();
    cuts.dedup();

    for (s, e, kind) in tokenize(text, lang) {
        let mut start = s;
        let mut points: Vec<usize> = cuts.iter().copied().filter(|c| *c > s && *c < e).collect();
        points.push(e);
        for end in points {
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                continue;
            }
            let var = var_ranges.iter().find(|(vs, ve, _)| start >= *vs && end <= *ve);
            let color = match (var, kind) {
                (Some((_, _, true)), _) => p.var_ok,
                (Some((_, _, false)), _) => p.var_bad,
                (None, Kind::Text) => p.text,
                (None, Kind::Str) => p.string,
                (None, Kind::Num) => p.number,
                (None, Kind::Kw) => p.keyword,
                (None, Kind::Comment) => p.comment,
                (None, Kind::Punct) => p.punct,
                (None, Kind::Tag) => p.tag,
            };
            let background = if search_ranges.iter().any(|(ss, se)| start >= *ss && end <= *se) {
                p.search
            } else {
                Color32::TRANSPARENT
            };
            let mut fmt = TextFormat { font_id: font.clone(), color, background, ..Default::default() };
            if var.is_some() {
                fmt.underline = egui::Stroke::new(1.0_f32, color.linear_multiply(0.6));
            }
            job.append(&text[start..end], 0.0, fmt);
            start = end;
        }
    }
    job
}

/// Pick a highlighter from a content type or raw language.
pub fn lang_for_content_type(ct: &str) -> Lang {
    let ct = ct.to_ascii_lowercase();
    if ct.contains("json") {
        Lang::Json
    } else if ct.contains("xml") || ct.contains("html") {
        Lang::Xml
    } else if ct.contains("javascript") {
        Lang::Js
    } else {
        Lang::Plain
    }
}

/// Pretty-print JSON or XML for the "Pretty" response view.
pub fn pretty(text: &str, lang: Lang) -> String {
    match lang {
        Lang::Json => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| text.to_string()),
        Lang::Xml => pretty_xml(text),
        _ => text.to_string(),
    }
}

fn pretty_xml(text: &str) -> String {
    if text.contains('\n') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut depth: usize = 0;
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.starts_with('<') {
            let end = rest.find('>').map(|p| p + 1).unwrap_or(rest.len());
            let tag = &rest[..end];
            let closing = tag.starts_with("</");
            let self_closing = tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!");
            if closing {
                depth = depth.saturating_sub(1);
            }
            out.push_str(&"  ".repeat(depth));
            out.push_str(tag);
            rest = &rest[end..];
            // Keep `<a>text</a>` on one line.
            if !closing && !self_closing {
                if let Some(next) = rest.find('<')
                    && next > 0
                    && rest[next..].starts_with("</")
                {
                    let close = rest.find('>').map(|p| p + 1).unwrap_or(rest.len());
                    out.push_str(&rest[..close]);
                    rest = &rest[close..];
                    out.push('\n');
                    continue;
                }
                depth += 1;
            }
            out.push('\n');
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let t = rest[..end].trim();
            if !t.is_empty() {
                out.push_str(&"  ".repeat(depth));
                out.push_str(t);
                out.push('\n');
            }
            rest = &rest[end..];
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Variable;

    #[test]
    fn json_tokens_cover_input() {
        let text = r#"{"a": [1, true, null], // c
"b": "x\"y"}"#;
        let toks = tokenize(text, Lang::Json);
        let rebuilt: String = toks.iter().map(|(s, e, _)| &text[*s..*e]).collect();
        assert_eq!(rebuilt, text, "tokens must cover every byte in order");
        assert!(toks.iter().any(|(s, e, k)| *k == Kind::Kw && &text[*s..*e] == "true"));
        assert!(toks.iter().any(|(s, e, k)| *k == Kind::Str && &text[*s..*e] == r#""x\"y""#));
        assert!(toks.iter().any(|(_, _, k)| *k == Kind::Comment));
    }

    #[test]
    fn layout_preserves_text_and_marks_variables() {
        let mut vars = VarContext::default();
        vars.globals.push(Variable::new("ok", "1"));
        let text = "{{ok}}/{{missing}}/é";
        let job = layout(text, Lang::Plain, FontId::monospace(12.0), false, Some(&vars), "MIS", 100.0);
        assert_eq!(job.text, text);
        let p = palette(false);
        let colors: Vec<_> = job.sections.iter().map(|s| s.format.color).collect();
        assert!(colors.contains(&p.var_ok) && colors.contains(&p.var_bad));
        assert!(job.sections.iter().any(|s| s.format.background == p.search));
    }

    #[test]
    fn pretty_printing() {
        assert_eq!(pretty(r#"{"a":1}"#, Lang::Json), "{\n  \"a\": 1\n}");
        assert_eq!(pretty("<a><b>x</b><c/></a>", Lang::Xml), "<a>\n  <b>x</b>\n  <c/>\n</a>");
        assert_eq!(pretty("not json", Lang::Json), "not json");
    }
}
