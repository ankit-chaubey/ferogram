// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

use ferogram_tl_types as tl;

pub(crate) fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut rem = s.trim();
    while !rem.is_empty() {
        let key_end = rem
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rem.len());
        let key = rem[..key_end].to_string();
        if key.is_empty() {
            rem = &rem[1..];
            continue;
        }
        rem = rem[key_end..].trim_start();
        if rem.starts_with('=') {
            rem = rem[1..].trim_start();
            if rem.starts_with('"') {
                let inner = &rem[1..];
                let close = inner.find('"').unwrap_or(inner.len());
                result.push((key, inner[..close].to_string()));
                rem = inner[close..].trim_start_matches('"').trim_start();
            } else if rem.starts_with('\'') {
                let inner = &rem[1..];
                let close = inner.find('\'').unwrap_or(inner.len());
                result.push((key, inner[..close].to_string()));
                rem = inner[close..].trim_start_matches('\'').trim_start();
            } else {
                let end = rem.find(char::is_whitespace).unwrap_or(rem.len());
                result.push((key, rem[..end].to_string()));
                rem = rem[end..].trim_start();
            }
        } else {
            // Boolean attribute (no `=`), e.g. `expandable`
            result.push((key, String::new()));
        }
    }
    result
}

pub(crate) fn parse_tag(s: &str) -> (&str, Vec<(String, String)>) {
    let mut parts = s.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").trim_end_matches('/');
    let attrs = parse_attrs(parts.next().unwrap_or(""));
    (name, attrs)
}

pub(crate) fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{00A0}")
}

// Rich Message parsers
//
// Convert Rich Markdown / Rich HTML into `Vec<tl::enums::PageBlock>`,
// which is the native MTProto representation used by `InputRichMessage`.
//
// Supported block types (both parsers):
//   Headings H1-H6, Paragraph, Pre/Code block, Divider (---), Anchor (<a name>),
//   Unordered list, Ordered list, Task list,
//   Blockquote, Pullquote/Aside,
//   Table (with thead, alignment, colspan, rowspan),
//   Details/Summary (collapsible),
//   Media: Photo, Video, Audio, VoiceNote, Animation,
//   Collage, Slideshow,
//   Map (<tg-map>),
//   Math block ($$…$$ / ```math / <tg-math-block>),
//   Footnotes ([^id]: …),
//   Footer.
//
// Supported inline (RichText) types:
//   Plain, Bold, Italic, Underline, Strike, Fixed (code), Marked (==),
//   Spoiler, Subscript, Superscript, Math (inline $…$),
//   URL, Email, Phone, MentionName, CustomEmoji, Date (tg://time),
//   Hashtag, Cashtag, BotCommand, AutoUrl, AutoEmail, AutoPhone, BankCard,
//   Anchor (textAnchor), Concat.

// Shared helpers

pub(crate) fn rt_empty() -> tl::enums::RichText {
    tl::enums::RichText::TextEmpty
}

pub(crate) fn rt_plain(s: impl Into<String>) -> tl::enums::RichText {
    let t = s.into();
    if t.is_empty() {
        return rt_empty();
    }
    tl::enums::RichText::TextPlain(tl::types::TextPlain { text: t })
}

pub(crate) fn rt_bold(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextBold(Box::new(tl::types::TextBold { text: inner }))
}

pub(crate) fn rt_italic(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextItalic(Box::new(tl::types::TextItalic { text: inner }))
}

pub(crate) fn rt_underline(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextUnderline(Box::new(tl::types::TextUnderline { text: inner }))
}

pub(crate) fn rt_strike(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextStrike(Box::new(tl::types::TextStrike { text: inner }))
}

pub(crate) fn rt_fixed(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextFixed(Box::new(tl::types::TextFixed { text: inner }))
}

pub(crate) fn rt_marked(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextMarked(Box::new(tl::types::TextMarked { text: inner }))
}

pub(crate) fn rt_spoiler(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextSpoiler(Box::new(tl::types::TextSpoiler { text: inner }))
}

pub(crate) fn rt_subscript(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextSubscript(Box::new(tl::types::TextSubscript { text: inner }))
}

pub(crate) fn rt_superscript(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextSuperscript(Box::new(tl::types::TextSuperscript { text: inner }))
}

pub(crate) fn rt_url(inner: tl::enums::RichText, url: String) -> tl::enums::RichText {
    tl::enums::RichText::TextUrl(Box::new(tl::types::TextUrl {
        text: inner,
        url,
        webpage_id: 0,
    }))
}

pub(crate) fn rt_email(inner: tl::enums::RichText, email: String) -> tl::enums::RichText {
    tl::enums::RichText::TextEmail(Box::new(tl::types::TextEmail { text: inner, email }))
}

pub(crate) fn rt_phone(inner: tl::enums::RichText, phone: String) -> tl::enums::RichText {
    tl::enums::RichText::TextPhone(Box::new(tl::types::TextPhone { text: inner, phone }))
}

pub(crate) fn rt_mention_name(inner: tl::enums::RichText, user_id: i64) -> tl::enums::RichText {
    tl::enums::RichText::TextMentionName(Box::new(tl::types::TextMentionName {
        text: inner,
        user_id,
    }))
}

pub(crate) fn rt_custom_emoji(document_id: i64, alt: String) -> tl::enums::RichText {
    tl::enums::RichText::TextCustomEmoji(tl::types::TextCustomEmoji { document_id, alt })
}

pub(crate) fn rt_math(source: String) -> tl::enums::RichText {
    tl::enums::RichText::TextMath(tl::types::TextMath { source })
}

pub(crate) fn rt_anchor(inner: tl::enums::RichText, name: String) -> tl::enums::RichText {
    tl::enums::RichText::TextAnchor(Box::new(tl::types::TextAnchor { text: inner, name }))
}

pub(crate) fn rt_date(inner: tl::enums::RichText, date: i32, fmt: &str) -> tl::enums::RichText {
    let (relative, short_time, long_time, short_date, long_date, day_of_week) =
        parse_tg_time_format(fmt);
    tl::enums::RichText::TextDate(Box::new(tl::types::TextDate {
        relative,
        short_time,
        long_time,
        short_date,
        long_date,
        day_of_week,
        text: inner,
        date,
    }))
}

pub(crate) fn rt_concat(parts: Vec<tl::enums::RichText>) -> tl::enums::RichText {
    let non_empty: Vec<_> = parts
        .into_iter()
        .filter(|r| !matches!(r, tl::enums::RichText::TextEmpty))
        .collect();
    match non_empty.len() {
        0 => rt_empty(),
        1 => non_empty.into_iter().next().unwrap(),
        _ => tl::enums::RichText::TextConcat(tl::types::TextConcat { texts: non_empty }),
    }
}

pub(crate) fn empty_caption() -> tl::enums::PageCaption {
    tl::enums::PageCaption::PageCaption(tl::types::PageCaption {
        text: rt_empty(),
        credit: rt_empty(),
    })
}

pub(crate) fn caption_text(text: tl::enums::RichText) -> tl::enums::PageCaption {
    tl::enums::PageCaption::PageCaption(tl::types::PageCaption {
        text,
        credit: rt_empty(),
    })
}

pub(crate) fn caption_text_credit(
    text: tl::enums::RichText,
    credit: tl::enums::RichText,
) -> tl::enums::PageCaption {
    tl::enums::PageCaption::PageCaption(tl::types::PageCaption { text, credit })
}

// Determine media type from URL (extension/mime heuristic)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Photo,
    Video,
    Audio,
    Voice,
    Animation,
}

pub(crate) fn media_kind_from_url(url: &str) -> MediaKind {
    let u = url.to_ascii_lowercase();
    let path = u.split('?').next().unwrap_or(&u);
    if path.ends_with(".ogg") || path.ends_with(".oga") {
        return MediaKind::Voice;
    }
    if path.ends_with(".mp3")
        || path.ends_with(".m4a")
        || path.ends_with(".flac")
        || path.ends_with(".wav")
    {
        return MediaKind::Audio;
    }
    if path.ends_with(".gif") {
        return MediaKind::Animation;
    }
    if path.ends_with(".mp4")
        || path.ends_with(".mov")
        || path.ends_with(".webm")
        || path.ends_with(".avi")
    {
        return MediaKind::Video;
    }
    if path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".png")
        || path.ends_with(".webp")
        || path.ends_with(".bmp")
    {
        return MediaKind::Photo;
    }
    MediaKind::Photo // default
}

// Build a PageBlock for a standalone media URL with optional caption and spoiler.
// Since RichMessage media is URL-based (not file_id-based), we use PageBlockEmbed
// for the actual URL and fall back to typed blocks with id=0 for semantic clarity.
// The Bot API rich message spec transmits media as URLs in InputRichFile which
// are resolved server-side; we model that with id=0 stubs so callers that
// use inputRichMessageHTML/inputRichMessageMarkdown paths work correctly.
pub(crate) fn media_block(
    url: &str,
    caption: tl::enums::PageCaption,
    spoiler: bool,
) -> tl::enums::PageBlock {
    let kind = media_kind_from_url(url);
    match kind {
        MediaKind::Photo => tl::enums::PageBlock::Photo(tl::types::PageBlockPhoto {
            spoiler,
            photo_id: 0,
            caption,
            url: Some(url.to_string()),
            webpage_id: None,
        }),
        MediaKind::Video | MediaKind::Animation => {
            tl::enums::PageBlock::Video(tl::types::PageBlockVideo {
                autoplay: false,
                r#loop: false,
                spoiler,
                video_id: 0,
                caption,
            })
        }
        MediaKind::Audio => tl::enums::PageBlock::Audio(tl::types::PageBlockAudio {
            audio_id: 0,
            caption,
        }),
        MediaKind::Voice => tl::enums::PageBlock::Audio(tl::types::PageBlockAudio {
            audio_id: 0,
            caption,
        }),
    }
}

// Inline RichText parser for Rich Markdown inline spans

/// Parse a Rich Markdown inline string (may contain nested `**`, `*`, `_`, `__`,
/// `~~`, `||`, `` ` ``, `==text==`, `[label](url)`, `![alt](tg://emoji?id=N)`,
/// `![alt](tg://time?unix=N&format=F)`, `$…$`, `<u>`, `<sub>`, `<sup>`,
/// `<tg-spoiler>`, `<ins>`, `<mark>`) into a `RichText` tree.
pub fn parse_rich_inline_md(text: &str) -> tl::enums::RichText {
    let chars: Vec<char> = text.chars().collect();
    let (parts, _) = parse_rich_inline_md_chars(&chars, 0, chars.len(), &[]);
    rt_concat(parts)
}

pub(crate) fn parse_rich_inline_md_chars(
    chars: &[char],
    start: usize,
    end: usize,
    stop_at: &[char],
) -> (Vec<tl::enums::RichText>, usize) {
    let mut parts: Vec<tl::enums::RichText> = Vec::new();
    let mut buf = String::new();
    let mut i = start;

    macro_rules! flush {
        () => {
            if !buf.is_empty() {
                parts.push(rt_plain(std::mem::take(&mut buf)));
            }
        };
    }

    while i < end {
        let c = chars[i];

        // Backslash escape
        if c == '\\' && i + 1 < end {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Stop chars (used for nested parsing, e.g. stop at `]`)
        if stop_at.contains(&c) {
            break;
        }

        // HTML inline tags embedded in markdown: <u>, <ins>, <sub>, <sup>, <tg-spoiler>, <mark>, <s>, <b>, <i>
        if c == '<'
            && let Some((new_i, rt)) = try_parse_html_inline_tag(chars, i, end)
        {
            flush!();
            parts.push(rt);
            i = new_i;
            continue;
        }

        // ``` inline code block (take all until next ```)
        if c == '`' && i + 2 < end && chars[i + 1] == '`' && chars[i + 2] == '`' {
            // triple backtick in inline context: treat as literal (block parsers handle code blocks)
            buf.push('`');
            buf.push('`');
            buf.push('`');
            i += 3;
            continue;
        }

        // Inline code: `…`
        if c == '`' {
            let mut j = i + 1;
            while j < end && chars[j] != '`' {
                j += 1;
            }
            if j < end {
                let code: String = chars[i + 1..j].iter().collect();
                flush!();
                parts.push(rt_fixed(rt_plain(code)));
                i = j + 1;
                continue;
            }
        }

        // Custom emoji: ![alt](tg://emoji?id=N) or tg://time?unix=N&format=F
        if c == '!'
            && i + 1 < end
            && chars[i + 1] == '['
            && let Some((j, url, alt)) = try_parse_md_link(chars, i + 1, end)
        {
            flush!();
            if let Some(rest) = url.strip_prefix("tg://emoji?id=")
                && let Ok(doc_id) = rest.parse::<i64>()
            {
                parts.push(rt_custom_emoji(doc_id, alt));
                i = j;
                continue;
            }
            if url.starts_with("tg://time?") || url.starts_with("tg://user?") {
                let inner = rt_plain(alt.clone());
                if let Some(p) = parse_tg_scheme(&url, inner, &alt) {
                    parts.push(p);
                    i = j;
                    continue;
                }
            }
            // Fallback: treat as text
            buf.push_str(&alt);
            i = j;
            continue;
        }

        // Inline link: [label](url)
        if c == '['
            && let Some((j, url, label)) = try_parse_md_link(chars, i, end)
        {
            flush!();
            let inner = parse_rich_inline_md(&label);
            let rt = build_link_rt(inner, &url, &label);
            parts.push(rt);
            i = j;
            continue;
        }

        // ==mark==
        if c == '=' && i + 1 < end && chars[i + 1] == '=' {
            let close = find_two_char_close(chars, i + 2, end, '=');
            if let Some(cl) = close {
                let inner_text: String = chars[i + 2..cl].iter().collect();
                flush!();
                parts.push(rt_marked(parse_rich_inline_md(&inner_text)));
                i = cl + 2;
                continue;
            }
        }

        // ||spoiler||
        if c == '|' && i + 1 < end && chars[i + 1] == '|' {
            let close = find_two_char_close(chars, i + 2, end, '|');
            if let Some(cl) = close {
                let inner_text: String = chars[i + 2..cl].iter().collect();
                flush!();
                parts.push(rt_spoiler(parse_rich_inline_md(&inner_text)));
                i = cl + 2;
                continue;
            }
        }

        // ~~strikethrough~~
        if c == '~' && i + 1 < end && chars[i + 1] == '~' {
            let close = find_two_char_close(chars, i + 2, end, '~');
            if let Some(cl) = close {
                let inner_text: String = chars[i + 2..cl].iter().collect();
                flush!();
                parts.push(rt_strike(parse_rich_inline_md(&inner_text)));
                i = cl + 2;
                continue;
            }
        }

        // **bold** (two stars)
        if c == '*'
            && i + 1 < end
            && chars[i + 1] == '*'
            && let Some(cl) = find_two_char_close(chars, i + 2, end, '*')
        {
            let inner_text: String = chars[i + 2..cl].iter().collect();
            flush!();
            parts.push(rt_bold(parse_rich_inline_md(&inner_text)));
            i = cl + 2;
            continue;
        }

        // *italic* (single star)
        if c == '*'
            && let Some(cl) = find_one_char_close(chars, i + 1, end, '*')
        {
            let inner_text: String = chars[i + 1..cl].iter().collect();
            flush!();
            parts.push(rt_italic(parse_rich_inline_md(&inner_text)));
            i = cl + 1;
            continue;
        }

        // __bold__ (two underscores = bold in rich markdown)
        if c == '_'
            && i + 1 < end
            && chars[i + 1] == '_'
            && let Some(cl) = find_two_char_close(chars, i + 2, end, '_')
        {
            let inner_text: String = chars[i + 2..cl].iter().collect();
            flush!();
            parts.push(rt_bold(parse_rich_inline_md(&inner_text)));
            i = cl + 2;
            continue;
        }

        // _italic_ (single underscore)
        if c == '_'
            && let Some(cl) = find_one_char_close(chars, i + 1, end, '_')
        {
            let inner_text: String = chars[i + 1..cl].iter().collect();
            flush!();
            parts.push(rt_italic(parse_rich_inline_md(&inner_text)));
            i = cl + 1;
            continue;
        }

        // Inline math: $source$
        if c == '$'
            && let Some(cl) = find_one_char_close(chars, i + 1, end, '$')
        {
            let src: String = chars[i + 1..cl].iter().collect();
            flush!();
            parts.push(rt_math(src));
            i = cl + 1;
            continue;
        }

        buf.push(c);
        i += 1;
    }

    flush!();
    (parts, i)
}

/// Try to find `..]] (close))` for `[label](url)` or `![label](url)`.
/// Returns `(next_i, url, label_text)`.
pub(crate) fn try_parse_md_link(
    chars: &[char],
    start: usize,
    end: usize,
) -> Option<(usize, String, String)> {
    // start points at `[`
    if start >= end || chars[start] != '[' {
        return None;
    }
    let mut depth = 1i32;
    let mut j = start + 1;
    while j < end {
        if chars[j] == '[' {
            depth += 1;
        } else if chars[j] == ']' {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        j += 1;
    }
    if j >= end || j + 1 >= end || chars[j + 1] != '(' {
        return None;
    }
    let label: String = chars[start + 1..j].iter().collect();
    let mut k = j + 2;
    // Handle quoted title: (url "title") - we skip title
    while k < end && chars[k] != ')' {
        k += 1;
    }
    if k >= end {
        return None;
    }
    let url_part: String = chars[j + 2..k].iter().collect();
    // Strip optional quoted title from url
    let url = strip_url_title(&url_part);
    Some((k + 1, url, label))
}

pub(crate) fn strip_url_title(s: &str) -> String {
    // "(url "title")" → url; we already stripped outer parens
    let s = s.trim();
    if let Some(q) = s.find(" \"") {
        return s[..q].trim().to_string();
    }
    if let Some(q) = s.find(" '") {
        return s[..q].trim().to_string();
    }
    s.to_string()
}

pub(crate) fn find_two_char_close(
    chars: &[char],
    from: usize,
    end: usize,
    ch: char,
) -> Option<usize> {
    let mut i = from;
    while i + 1 < end {
        if chars[i] == ch && chars[i + 1] == ch {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub(crate) fn find_one_char_close(
    chars: &[char],
    from: usize,
    end: usize,
    ch: char,
) -> Option<usize> {
    let mut i = from;
    while i < end {
        if chars[i] == ch {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub(crate) fn build_link_rt(
    inner: tl::enums::RichText,
    url: &str,
    label: &str,
) -> tl::enums::RichText {
    if let Some(rest) = url.strip_prefix("tg://user?id=")
        && let Ok(uid) = rest.parse::<i64>()
    {
        return rt_mention_name(inner, uid);
    }
    if let Some(email_raw) = url.strip_prefix("mailto:") {
        let email = email_raw.to_string();
        return rt_email(inner, email);
    }
    if let Some(phone_raw) = url.strip_prefix("tel:") {
        let phone = phone_raw.to_string();
        return rt_phone(inner, phone);
    }
    if url.starts_with('#') {
        // In-document anchor link - treat as anchor reference
        return rt_url(inner, url.to_string());
    }
    if let Some(p) = parse_tg_scheme(url, inner.clone(), label) {
        return p;
    }
    rt_url(inner, url.to_string())
}

pub(crate) fn parse_tg_scheme(
    url: &str,
    inner: tl::enums::RichText,
    _label: &str,
) -> Option<tl::enums::RichText> {
    if url.starts_with("tg://time?") || url.starts_with("tg://time?unix=") {
        // Parse unix= and format=
        let params: std::collections::HashMap<_, _> = url
            .split('?')
            .nth(1)
            .unwrap_or("")
            .split('&')
            .filter_map(|kv| {
                let mut it = kv.splitn(2, '=');
                Some((it.next()?, it.next()?))
            })
            .collect();
        let unix: i32 = params.get("unix").and_then(|v| v.parse().ok()).unwrap_or(0);
        let fmt = params.get("format").copied().unwrap_or("t");
        return Some(rt_date(inner, unix, fmt));
    }
    None
}

/// Try to parse an HTML inline tag starting at `chars[i]` (which is `<`).
/// Returns `(next_i, RichText)` or `None` if not a recognised inline tag.
pub(crate) fn try_parse_html_inline_tag(
    chars: &[char],
    i: usize,
    end: usize,
) -> Option<(usize, tl::enums::RichText)> {
    // Find close `>`
    let mut j = i + 1;
    while j < end && chars[j] != '>' {
        j += 1;
    }
    if j >= end {
        return None;
    }
    let tag_raw: String = chars[i + 1..j].iter().collect();
    let is_self_closing = tag_raw.ends_with('/');
    let tag_clean = tag_raw.trim_end_matches('/').trim();
    let (tag_name, attrs) = parse_tag(tag_clean);
    let after_open = j + 1;

    // Self-closing / void tags
    if is_self_closing || matches!(tag_name, "br") {
        if tag_name == "br" {
            return Some((after_open, rt_plain("\n")));
        }
        // <tg-map>, <img> etc - handled at block level, skip here
        return None;
    }

    // Recognised inline container tags
    let wrap: Option<fn(tl::enums::RichText) -> tl::enums::RichText> = match tag_name {
        "b" | "strong" => Some(rt_bold),
        "i" | "em" => Some(rt_italic),
        "u" | "ins" => Some(rt_underline),
        "s" | "del" | "strike" => Some(rt_strike),
        "code" => Some(rt_fixed),
        "mark" => Some(rt_marked),
        "tg-spoiler" => Some(rt_spoiler),
        "sub" => Some(rt_subscript),
        "sup" => Some(rt_superscript),
        _ => None,
    };

    if let Some(wrap_fn) = wrap {
        // Find close tag
        let close_tag = format!("</{tag_name}>");
        let content_str: String = chars[after_open..].iter().collect();
        if let Some(cl) = content_str.find(&close_tag) {
            let inner_str: String = chars[after_open..after_open + cl].iter().collect();
            let inner = parse_rich_inline_md(&inner_str);
            let next_i = after_open + cl + close_tag.len();
            return Some((next_i, wrap_fn(inner)));
        }
        return None;
    }

    // <tg-time unix="N" format="F">label</tg-time>
    if tag_name == "tg-time" {
        let unix: i32 = attrs
            .iter()
            .find(|(k, _)| k == "unix")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let fmt = attrs
            .iter()
            .find(|(k, _)| k == "format")
            .map(|(_, v)| v.as_str())
            .unwrap_or("t");
        let close = "</tg-time>";
        let content_str: String = chars[after_open..].iter().collect();
        if let Some(cl) = content_str.find(close) {
            let label_str: String = chars[after_open..after_open + cl].iter().collect();
            let inner = rt_plain(label_str);
            return Some((after_open + cl + close.len(), rt_date(inner, unix, fmt)));
        }
        return None;
    }

    // <tg-emoji emoji-id="N">alt</tg-emoji>
    if tag_name == "tg-emoji" {
        let doc_id: i64 = attrs
            .iter()
            .find(|(k, _)| k == "emoji-id")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let close = "</tg-emoji>";
        let content_str: String = chars[after_open..].iter().collect();
        if let Some(cl) = content_str.find(close) {
            let alt: String = chars[after_open..after_open + cl].iter().collect();
            return Some((after_open + cl + close.len(), rt_custom_emoji(doc_id, alt)));
        }
        return None;
    }

    // <a href="…"> link </a>
    if tag_name == "a" {
        let href = attrs
            .iter()
            .find(|(k, _)| k == "href")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let name_attr = attrs
            .iter()
            .find(|(k, _)| k == "name")
            .map(|(_, v)| v.clone());
        if href.is_empty()
            && let Some(name) = name_attr
        {
            // Anchor definition: <a name="chapter-1"></a>
            let close = "</a>";
            let content_str: String = chars[after_open..].iter().collect();
            if let Some(cl) = content_str.find(close) {
                return Some((after_open + cl + close.len(), rt_anchor(rt_empty(), name)));
            }
            return None;
        }
        let close = "</a>";
        let content_str: String = chars[after_open..].iter().collect();
        if let Some(cl) = content_str.find(close) {
            let label_str: String = chars[after_open..after_open + cl].iter().collect();
            let inner = parse_rich_inline_md(&label_str);
            let rt = build_link_rt(inner, &href, &label_str);
            return Some((after_open + cl + close.len(), rt));
        }
        return None;
    }

    // <tg-reference name="…">text</tg-reference>
    if tag_name == "tg-reference" {
        let name = attrs
            .iter()
            .find(|(k, _)| k == "name")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let close = "</tg-reference>";
        let content_str: String = chars[after_open..].iter().collect();
        if let Some(cl) = content_str.find(close) {
            let label_str: String = chars[after_open..after_open + cl].iter().collect();
            let inner = parse_rich_inline_md(&label_str);
            return Some((after_open + cl + close.len(), rt_anchor(inner, name)));
        }
        return None;
    }

    // <tg-math>source</tg-math>
    if tag_name == "tg-math" {
        let close = "</tg-math>";
        let content_str: String = chars[after_open..].iter().collect();
        if let Some(cl) = content_str.find(close) {
            let src: String = chars[after_open..after_open + cl].iter().collect();
            return Some((after_open + cl + close.len(), rt_math(src)));
        }
        return None;
    }

    None
}

pub(crate) fn parse_tg_time_format(fmt: &str) -> (bool, bool, bool, bool, bool, bool) {
    let relative = fmt.contains('r') || fmt.contains('R');
    let long_time = fmt.contains("tt") || fmt.contains('T');
    let short_time = !long_time && fmt.contains('t');
    let long_date = fmt.contains('D');
    let short_date = !long_date && fmt.contains('d');
    let day_of_week = fmt.contains('w') || fmt.contains('W');
    (
        relative,
        short_time,
        long_time,
        short_date,
        long_date,
        day_of_week,
    )
}

pub(crate) fn tg_time_flags_to_format(e: &tl::types::MessageEntityFormattedDate) -> String {
    let mut s = String::new();
    if e.day_of_week {
        s.push('w');
    }
    if e.long_date {
        s.push('D');
    } else if e.short_date {
        s.push('d');
    }
    if e.long_time {
        s.push_str("tt");
    } else if e.short_time {
        s.push('t');
    }
    if e.relative {
        s.push('r');
    }
    if s.is_empty() {
        s.push('t');
    } // sensible default
    s
}

pub(crate) fn heading_block(level: usize, rt: tl::enums::RichText) -> tl::enums::PageBlock {
    match level {
        1 => tl::enums::PageBlock::Heading1(tl::types::PageBlockHeading1 { text: rt }),
        2 => tl::enums::PageBlock::Heading2(tl::types::PageBlockHeading2 { text: rt }),
        3 => tl::enums::PageBlock::Heading3(tl::types::PageBlockHeading3 { text: rt }),
        4 => tl::enums::PageBlock::Heading4(tl::types::PageBlockHeading4 { text: rt }),
        5 => tl::enums::PageBlock::Heading5(tl::types::PageBlockHeading5 { text: rt }),
        _ => tl::enums::PageBlock::Heading6(tl::types::PageBlockHeading6 { text: rt }),
    }
}

pub(crate) fn split_cite(html: &str) -> (String, String) {
    if let Some(cite_start) = html.to_ascii_lowercase().find("<cite>") {
        let text = html[..cite_start].to_string();
        let after = &html[cite_start + "<cite>".len()..];
        let credit = after
            .to_ascii_lowercase()
            .find("</cite>")
            .map(|i| after[..i].to_string())
            .unwrap_or_else(|| after.to_string());
        (text, credit)
    } else {
        (html.to_string(), String::new())
    }
}

pub(crate) fn extract_between(s: &str, open: &str, close: &str) -> Option<String> {
    let lo = s.to_ascii_lowercase();
    let start = lo.find(&open.to_ascii_lowercase())? + open.len();
    let end = lo[start..]
        .find(&close.to_ascii_lowercase())
        .map(|i| start + i)?;
    Some(s[start..end].to_string())
}

pub(crate) fn extract_src_from_figure(html: &str) -> Option<String> {
    for part in html.split('<') {
        if (part.starts_with("img ") || part.starts_with("video ") || part.starts_with("audio "))
            && let Some(src) = extract_attr_value(part, "src")
        {
            return Some(src);
        }
    }
    None
}

pub(crate) fn extract_collage_items(
    html: &str,
) -> (Vec<tl::enums::PageBlock>, Option<tl::enums::PageCaption>) {
    let mut items = Vec::new();
    // Find all <img src="…"/> and <video src="…"/>
    for part in html.split('<') {
        if part.starts_with("img ") || part.starts_with("video ") {
            let src = extract_attr_value(part, "src");
            if let Some(url) = src {
                items.push(media_block(&url, empty_caption(), false));
            }
        }
    }
    let cap = extract_between(html, "<figcaption>", "</figcaption>").map(|c| {
        let (t, cr) = split_cite(&c);
        caption_text_credit(parse_rich_inline_md(&t), parse_rich_inline_md(&cr))
    });
    (items, cap)
}

pub(crate) fn parse_html_list_items(html: &str, _ordered: bool) -> Vec<tl::enums::PageListItem> {
    let mut items = Vec::new();
    let lo = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(li_start) = lo[search..].find("<li") {
        let li_start = search + li_start;
        let after_open = lo[li_start..]
            .find('>')
            .map(|i| li_start + i + 1)
            .unwrap_or(html.len());
        let li_end = lo[after_open..]
            .find("</li>")
            .map(|i| after_open + i)
            .unwrap_or(html.len());
        let content = &html[after_open..li_end];
        let (_, attrs_raw) = parse_tag(html[li_start + 1..].split('>').next().unwrap_or("").trim());
        let checked_attr: Option<bool> =
            if html[li_start..].to_ascii_lowercase().starts_with("<li ") {
                let li_tag = html[li_start..].split('>').next().unwrap_or("");
                if li_tag.to_ascii_lowercase().contains("checked") {
                    Some(true)
                } else if li_tag.to_ascii_lowercase().contains("checkbox") {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            };
        let _ = attrs_raw;
        items.push(tl::enums::PageListItem::Text(tl::types::PageListItemText {
            checkbox: checked_attr.is_some(),
            checked: checked_attr.unwrap_or(false),
            text: parse_rich_inline_md(content),
        }));
        search = li_end + 5;
    }
    items
}

pub(crate) fn parse_html_ordered_list_items(
    html: &str,
    list_type: Option<&str>,
) -> Vec<tl::enums::PageListOrderedItem> {
    let mut items = Vec::new();
    let lo = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(li_start) = lo[search..].find("<li") {
        let li_start = search + li_start;
        let after_tag = lo[li_start..]
            .find('>')
            .map(|i| li_start + i)
            .unwrap_or(html.len());
        let tag_attrs_raw = html[li_start + 1..after_tag].trim();
        let (_, attrs) = parse_tag(tag_attrs_raw);
        let value: Option<i32> = attrs
            .iter()
            .find(|(k, _)| k == "value")
            .and_then(|(_, v)| v.parse().ok());
        let item_type: Option<String> = attrs
            .iter()
            .find(|(k, _)| k == "type")
            .map(|(_, v)| v.clone())
            .or_else(|| list_type.map(|s| s.to_string()));
        let after_open = after_tag + 1;
        let li_end = lo[after_open..]
            .find("</li>")
            .map(|i| after_open + i)
            .unwrap_or(html.len());
        let content = &html[after_open..li_end];
        items.push(tl::enums::PageListOrderedItem::Text(
            tl::types::PageListOrderedItemText {
                checkbox: false,
                checked: false,
                num: None,
                text: parse_rich_inline_md(content),
                value,
                r#type: item_type,
            },
        ));
        search = li_end + 5;
    }
    items
}

pub(crate) fn parse_html_table(html: &str) -> (tl::enums::RichText, Vec<tl::enums::PageTableRow>) {
    let lo = html.to_ascii_lowercase();
    // Extract caption
    let title = extract_between(html, "<caption>", "</caption>")
        .map(|c| parse_rich_inline_md(&c))
        .unwrap_or_else(rt_empty);

    let mut rows = Vec::new();
    let mut search = 0;
    while let Some(tr_start) = lo[search..].find("<tr") {
        let tr_start = search + tr_start;
        let after_tr = lo[tr_start..]
            .find('>')
            .map(|i| tr_start + i + 1)
            .unwrap_or(html.len());
        let tr_end = lo[after_tr..]
            .find("</tr>")
            .map(|i| after_tr + i)
            .unwrap_or(html.len());
        let row_html = &html[after_tr..tr_end];
        let cells = parse_html_table_cells(row_html);
        rows.push(tl::enums::PageTableRow::PageTableRow(
            tl::types::PageTableRow { cells },
        ));
        search = tr_end + 5;
    }
    (title, rows)
}

pub(crate) fn parse_html_table_cells(html: &str) -> Vec<tl::enums::PageTableCell> {
    let mut cells = Vec::new();
    let lo = html.to_ascii_lowercase();
    let mut search = 0;

    loop {
        let th_pos = lo[search..].find("<th").map(|i| (search + i, true));
        let td_pos = lo[search..].find("<td").map(|i| (search + i, false));
        let (cell_start, is_header) = match (th_pos, td_pos) {
            (Some(a), Some(b)) => {
                if a.0 <= b.0 {
                    a
                } else {
                    b
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        let close_tag = if is_header { "</th>" } else { "</td>" };
        let after_open = lo[cell_start..]
            .find('>')
            .map(|i| cell_start + i)
            .unwrap_or(html.len());
        let tag_raw = html[cell_start + 1..after_open].trim();
        let (_, attrs) = parse_tag(tag_raw);
        let colspan: Option<i32> = attrs
            .iter()
            .find(|(k, _)| k == "colspan")
            .and_then(|(_, v)| v.parse().ok());
        let rowspan: Option<i32> = attrs
            .iter()
            .find(|(k, _)| k == "rowspan")
            .and_then(|(_, v)| v.parse().ok());
        let align = attrs
            .iter()
            .find(|(k, _)| k == "align")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let valign = attrs
            .iter()
            .find(|(k, _)| k == "valign")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let align_center = align == "center";
        let align_right = align == "right";
        let valign_middle = valign == "middle";
        let valign_bottom = valign == "bottom";
        let content_start = after_open + 1;
        let cell_end = lo[content_start..]
            .find(close_tag)
            .map(|i| content_start + i)
            .unwrap_or(html.len());
        let content = &html[content_start..cell_end];
        cells.push(tl::enums::PageTableCell::PageTableCell(
            tl::types::PageTableCell {
                header: is_header,
                align_center,
                align_right,
                valign_middle,
                valign_bottom,
                text: Some(parse_rich_inline_md(content)),
                colspan,
                rowspan,
            },
        ));
        search = cell_end + close_tag.len();
    }
    cells
}

pub(crate) fn is_block_html_tag(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    for tag in &[
        "<details",
        "<tg-collage",
        "<tg-slideshow",
        "<aside",
        "<tg-math-block",
        "<footer",
        "<tg-map",
        "<figure",
        "<blockquote",
        "<h1",
        "<h2",
        "<h3",
        "<h4",
        "<h5",
        "<h6",
        "<p>",
        "<p ",
        "<pre",
        "<hr",
        "<ul",
        "<ol",
        "<table",
    ] {
        if lower.starts_with(tag) {
            return true;
        }
    }
    false
}

pub(crate) fn extract_title_from_url_part(s: &str) -> String {
    // url "title" → title
    if let Some(q) = s.find(" \"") {
        let after = &s[q + 2..];
        if let Some(close) = after.rfind('"') {
            return after[..close].to_string();
        }
    }
    String::new()
}

pub(crate) fn extract_tag_body(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    // find end of opening tag
    let start = html.to_ascii_lowercase().find(&open).unwrap_or(0);
    let after_open = html[start..]
        .find('>')
        .map(|i| start + i + 1)
        .unwrap_or(html.len());
    let end = html
        .to_ascii_lowercase()
        .rfind(&close)
        .unwrap_or(html.len());
    html[after_open.min(end)..end].to_string()
}

pub(crate) fn extract_attr_value(tag_body: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag_body.find(&needle)? + needle.len();
    let end = tag_body[start..].find('"')? + start;
    Some(tag_body[start..end].to_string())
}

pub(crate) fn extract_pre_content(html: &str) -> (String, String) {
    // <pre><code class="language-X">…</code></pre>
    if let Some(lang) = extract_between(html, "class=\"language-", "\"") {
        let code = extract_between(html, ">", "</code>")
            .or_else(|| extract_between(html, "<code", "</code>"))
            .unwrap_or_default();
        // Strip the class= part from code
        let code = code
            .trim_start_matches(|c: char| c != '>')
            .strip_prefix('>')
            .unwrap_or(&code)
            .to_string();
        return (lang, code);
    }
    let code = extract_tag_body(html, "pre");
    (String::new(), code)
}

pub(crate) fn list_unordered_start(line: &str) -> bool {
    let t = line.trim_start();
    (t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ "))
        && !matches!(t, "---" | "***" | "___")
}

pub(crate) fn list_ordered_start(line: &str) -> bool {
    let t = line.trim_start();
    let bytes = t.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return false;
    }
    let mut j = 0;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    j < bytes.len() && (bytes[j] == b'.' || bytes[j] == b')')
}

pub(crate) fn parse_list_item_unordered(line: &str) -> Option<(Option<bool>, &str)> {
    let t = line.trim_start();
    let rest = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))?;
    if let Some(r) = rest.strip_prefix("[ ] ") {
        return Some((Some(false), r));
    }
    if let Some(r) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        return Some((Some(true), r));
    }
    Some((None, rest))
}

pub(crate) fn parse_list_item_ordered(line: &str) -> Option<(i32, &str)> {
    let t = line.trim_start();
    let dot = t.find(['.', ')'])?;
    let num: i32 = t[..dot].parse().ok()?;
    let rest = t[dot + 1..].trim_start();
    Some((num, rest))
}

pub(crate) fn split_table_row(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').collect()
}

pub(crate) fn try_parse_media_line(line: &str) -> Option<tl::enums::PageBlock> {
    // ![](url) or ![](url "caption")
    if !line.starts_with("![](") {
        return None;
    }
    let inner = line.strip_prefix("![](")?.strip_suffix(')')?;
    let url = strip_url_title(inner);
    let cap_title = extract_title_from_url_part(inner);
    let cap = if cap_title.is_empty() {
        empty_caption()
    } else {
        caption_text(parse_rich_inline_md(&cap_title))
    };
    Some(media_block(&url, cap, false))
}
