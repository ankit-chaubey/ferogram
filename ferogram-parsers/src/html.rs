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

use crate::rich_common::{
    decode_html_entities, parse_tag, parse_tg_time_format, tg_time_flags_to_format,
};
use ferogram_tl_types as tl;

#[cfg(not(feature = "html5ever"))]
pub fn parse_html(html: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    let mut out = String::with_capacity(html.len());
    let mut ents = Vec::new();
    let mut stack: Vec<(HtmlTag, i32, Option<String>)> = Vec::new();
    let mut utf16_off: i32 = 0;

    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            let tag_start = i + 1;
            let mut j = tag_start;
            while j < len && bytes[j] != b'>' {
                j += 1;
            }
            let tag_content = &html[tag_start..j];
            i = j + 1;

            let is_close = tag_content.starts_with('/');
            let tag_str = if is_close {
                tag_content[1..].trim()
            } else {
                tag_content.trim()
            };
            let (tag_name, attrs) = parse_tag(tag_str);

            if is_close {
                // </code> inside <pre>: just pop the CodeInPre marker
                if tag_name == "code"
                    && let Some(pos) = stack
                        .iter()
                        .rposition(|(t, _, _)| matches!(t, HtmlTag::CodeInPre))
                {
                    stack.remove(pos);
                    continue;
                }

                if let Some(pos) = stack.iter().rposition(|(t, _, _)| t.closes_with(tag_name)) {
                    let (htag, start_off, extra) = stack.remove(pos);
                    let length = utf16_off - start_off;
                    if length > 0 {
                        let entity: Option<tl::enums::MessageEntity> = match htag {
                            HtmlTag::Bold => Some(tl::enums::MessageEntity::Bold(
                                tl::types::MessageEntityBold {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::Italic => Some(tl::enums::MessageEntity::Italic(
                                tl::types::MessageEntityItalic {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::Underline => Some(tl::enums::MessageEntity::Underline(
                                tl::types::MessageEntityUnderline {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::Strike => Some(tl::enums::MessageEntity::Strike(
                                tl::types::MessageEntityStrike {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::Spoiler | HtmlTag::SpanSpoiler => {
                                Some(tl::enums::MessageEntity::Spoiler(
                                    tl::types::MessageEntitySpoiler {
                                        offset: start_off,
                                        length,
                                    },
                                ))
                            }
                            HtmlTag::Code => Some(tl::enums::MessageEntity::Code(
                                tl::types::MessageEntityCode {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::CodeInPre => None,
                            HtmlTag::Pre => {
                                Some(tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
                                    offset: start_off,
                                    length,
                                    language: extra.unwrap_or_default(),
                                }))
                            }
                            HtmlTag::Link(url) => {
                                const PFX: &str = "tg://user?id=";
                                if let Some(s) = url.strip_prefix(PFX) {
                                    s.parse::<i64>().ok().map(|uid| {
                                        tl::enums::MessageEntity::MentionName(
                                            tl::types::MessageEntityMentionName {
                                                offset: start_off,
                                                length,
                                                user_id: uid,
                                            },
                                        )
                                    })
                                } else {
                                    Some(tl::enums::MessageEntity::TextUrl(
                                        tl::types::MessageEntityTextUrl {
                                            offset: start_off,
                                            length,
                                            url,
                                        },
                                    ))
                                }
                            }
                            HtmlTag::CustomEmoji(id) => {
                                Some(tl::enums::MessageEntity::CustomEmoji(
                                    tl::types::MessageEntityCustomEmoji {
                                        offset: start_off,
                                        length,
                                        document_id: id,
                                    },
                                ))
                            }
                            HtmlTag::Blockquote { collapsed } => {
                                Some(tl::enums::MessageEntity::Blockquote(
                                    tl::types::MessageEntityBlockquote {
                                        collapsed,
                                        offset: start_off,
                                        length,
                                    },
                                ))
                            }
                            HtmlTag::TgTime {
                                unix,
                                relative,
                                short_time,
                                long_time,
                                short_date,
                                long_date,
                                day_of_week,
                            } => Some(tl::enums::MessageEntity::FormattedDate(
                                tl::types::MessageEntityFormattedDate {
                                    relative,
                                    short_time,
                                    long_time,
                                    short_date,
                                    long_date,
                                    day_of_week,
                                    offset: start_off,
                                    length,
                                    date: unix,
                                },
                            )),
                            HtmlTag::Unknown => None,
                        };
                        if let Some(e) = entity {
                            ents.push(e);
                        }
                    }
                }
            } else {
                // Open tag
                let htag: HtmlTag = match tag_name {
                    "b" | "strong" => HtmlTag::Bold,
                    "i" | "em" => HtmlTag::Italic,
                    "u" | "ins" => HtmlTag::Underline, // <ins> is the underline alias
                    "s" | "del" | "strike" => HtmlTag::Strike,
                    "tg-spoiler" => HtmlTag::Spoiler,
                    "span" => {
                        if attrs.iter().any(|(k, v)| k == "class" && v == "tg-spoiler") {
                            HtmlTag::SpanSpoiler
                        } else {
                            HtmlTag::Unknown
                        }
                    }
                    "blockquote" => {
                        let collapsed = attrs.iter().any(|(k, _)| k == "expandable");
                        HtmlTag::Blockquote { collapsed }
                    }
                    "tg-time" => {
                        let unix: i32 = attrs
                            .iter()
                            .find(|(k, _)| k == "unix")
                            .and_then(|(_, v)| v.parse::<i32>().ok())
                            .unwrap_or(0);
                        let fmt = attrs
                            .iter()
                            .find(|(k, _)| k == "format")
                            .map(|(_, v)| v.as_str())
                            .unwrap_or("");
                        let (relative, short_time, long_time, short_date, long_date, day_of_week) =
                            parse_tg_time_format(fmt);
                        HtmlTag::TgTime {
                            unix,
                            relative,
                            short_time,
                            long_time,
                            short_date,
                            long_date,
                            day_of_week,
                        }
                    }
                    "code" => {
                        // Inside open <pre>: annotate language, push CodeInPre sentinel
                        if let Some(last) = stack.last_mut()
                            && matches!(last.0, HtmlTag::Pre)
                        {
                            let lang = attrs
                                .iter()
                                .find(|(k, _)| k == "class")
                                .and_then(|(_, v)| v.strip_prefix("language-"))
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            last.2 = Some(lang);
                            stack.push((HtmlTag::CodeInPre, utf16_off, None));
                            continue;
                        }
                        HtmlTag::Code
                    }
                    "pre" => HtmlTag::Pre,
                    "a" => HtmlTag::Link(
                        attrs
                            .iter()
                            .find(|(k, _)| k == "href")
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default(),
                    ),
                    "tg-emoji" => HtmlTag::CustomEmoji(
                        attrs
                            .iter()
                            .find(|(k, _)| k == "emoji-id")
                            .and_then(|(_, v)| v.parse::<i64>().ok())
                            .unwrap_or(0),
                    ),
                    "br" => {
                        out.push('\n');
                        utf16_off += 1;
                        continue;
                    }
                    _ => HtmlTag::Unknown,
                };
                stack.push((htag, utf16_off, None));
            }
        } else {
            let text_start = i;
            while i < len && bytes[i] != b'<' {
                i += 1;
            }
            let decoded = decode_html_entities(&html[text_start..i]);
            for ch in decoded.chars() {
                out.push(ch);
                utf16_off += ch.len_utf16() as i32;
            }
        }
    }

    (out, ents)
}

#[cfg(not(feature = "html5ever"))]
#[cfg(not(feature = "html5ever"))]
/// Parse HTML attributes including boolean attributes (e.g. `expandable`).
#[cfg(not(feature = "html5ever"))]
#[cfg(not(feature = "html5ever"))]
#[derive(Debug, Clone)]
enum HtmlTag {
    Bold,
    Italic,
    Underline,
    Strike,
    Spoiler,
    SpanSpoiler,
    Code,
    CodeInPre,
    Pre,
    Link(String),
    CustomEmoji(i64),
    Blockquote {
        collapsed: bool,
    },
    TgTime {
        unix: i32,
        relative: bool,
        short_time: bool,
        long_time: bool,
        short_date: bool,
        long_date: bool,
        day_of_week: bool,
    },
    Unknown,
}

#[cfg(not(feature = "html5ever"))]
impl HtmlTag {
    /// Returns true if this open tag is closed by the given close-tag name.
    /// Handles all Telegram HTML aliases: strong/b, em/i, ins/u, del|strike/s.
    fn closes_with(&self, tag_name: &str) -> bool {
        match self {
            Self::Bold => matches!(tag_name, "b" | "strong"),
            Self::Italic => matches!(tag_name, "i" | "em"),
            Self::Underline => matches!(tag_name, "u" | "ins"),
            Self::Strike => matches!(tag_name, "s" | "del" | "strike"),
            Self::Spoiler => tag_name == "tg-spoiler",
            Self::SpanSpoiler => tag_name == "span",
            Self::Code => tag_name == "code",
            Self::CodeInPre => false,
            Self::Pre => tag_name == "pre",
            Self::Link(_) => tag_name == "a",
            Self::CustomEmoji(_) => tag_name == "tg-emoji",
            Self::Blockquote { .. } => tag_name == "blockquote",
            Self::TgTime { .. } => tag_name == "tg-time",
            Self::Unknown => false,
        }
    }
}

// HTML parser: html5ever backend

/// Parse a Telegram-compatible HTML string into `(plain_text, entities)`.
///
/// Uses the [`html5ever`] spec-compliant tokenizer.
/// Enable the `html5ever` Cargo feature to activate this implementation.
#[cfg(feature = "html5ever")]
#[cfg_attr(docsrs, doc(cfg(feature = "html5ever")))]
pub fn parse_html(html: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    use html5ever::tendril::StrTendril;
    use html5ever::tokenizer::{
        BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer,
    };
    use std::cell::Cell;

    struct Sink {
        text: Cell<String>,
        entities: Cell<Vec<tl::enums::MessageEntity>>,
        offset: Cell<i32>,
    }

    impl TokenSink for Sink {
        type Handle = ();

        fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
            let mut text = self.text.take();
            let mut entities = self.entities.take();
            let mut offset = self.offset.get();

            macro_rules! close_ent {
                ($kind:ident) => {{
                    if let Some(idx) = entities
                        .iter()
                        .rposition(|e| matches!(e, tl::enums::MessageEntity::$kind(_)))
                    {
                        let closed_len = {
                            if let tl::enums::MessageEntity::$kind(ref mut inner) = entities[idx] {
                                inner.length = offset - inner.offset;
                                inner.length
                            } else {
                                unreachable!()
                            }
                        };
                        if closed_len == 0 {
                            entities.remove(idx);
                        }
                    }
                }};
            }

            match token {
                Token::TagToken(Tag {
                    kind: TagKind::StartTag,
                    name,
                    attrs,
                    ..
                }) => {
                    let len0 = 0i32;
                    match name.as_ref() {
                        "b" | "strong" => entities.push(tl::enums::MessageEntity::Bold(
                            tl::types::MessageEntityBold {
                                offset,
                                length: len0,
                            },
                        )),
                        "i" | "em" => entities.push(tl::enums::MessageEntity::Italic(
                            tl::types::MessageEntityItalic {
                                offset,
                                length: len0,
                            },
                        )),
                        "u" | "ins" => entities.push(tl::enums::MessageEntity::Underline(
                            tl::types::MessageEntityUnderline {
                                offset,
                                length: len0,
                            },
                        )),
                        "s" | "del" | "strike" => entities.push(tl::enums::MessageEntity::Strike(
                            tl::types::MessageEntityStrike {
                                offset,
                                length: len0,
                            },
                        )),
                        "tg-spoiler" => entities.push(tl::enums::MessageEntity::Spoiler(
                            tl::types::MessageEntitySpoiler {
                                offset,
                                length: len0,
                            },
                        )),
                        "span" => {
                            let is_spoiler = attrs.iter().any(|a| {
                                a.name.local.as_ref() == "class" && a.value.as_ref() == "tg-spoiler"
                            });
                            if is_spoiler {
                                entities.push(tl::enums::MessageEntity::Spoiler(
                                    tl::types::MessageEntitySpoiler {
                                        offset,
                                        length: len0,
                                    },
                                ));
                            }
                        }
                        "blockquote" => {
                            let collapsed =
                                attrs.iter().any(|a| a.name.local.as_ref() == "expandable");
                            entities.push(tl::enums::MessageEntity::Blockquote(
                                tl::types::MessageEntityBlockquote {
                                    collapsed,
                                    offset,
                                    length: len0,
                                },
                            ));
                        }
                        "tg-time" => {
                            let unix: i32 = attrs
                                .iter()
                                .find(|a| a.name.local.as_ref() == "unix")
                                .and_then(|a| a.value.as_ref().parse::<i32>().ok())
                                .unwrap_or(0);
                            let fmt = attrs
                                .iter()
                                .find(|a| a.name.local.as_ref() == "format")
                                .map(|a| a.value.as_ref().to_string())
                                .unwrap_or_default();
                            let (
                                relative,
                                short_time,
                                long_time,
                                short_date,
                                long_date,
                                day_of_week,
                            ) = parse_tg_time_format(&fmt);
                            entities.push(tl::enums::MessageEntity::FormattedDate(
                                tl::types::MessageEntityFormattedDate {
                                    relative,
                                    short_time,
                                    long_time,
                                    short_date,
                                    long_date,
                                    day_of_week,
                                    offset,
                                    length: len0,
                                    date: unix,
                                },
                            ));
                        }
                        "code" => {
                            let in_pre = entities.last().map_or(
                                false,
                                |e| matches!(e, tl::enums::MessageEntity::Pre(p) if p.length == 0),
                            );
                            if in_pre {
                                let lang = attrs
                                    .iter()
                                    .find(|a| a.name.local.as_ref() == "class")
                                    .and_then(|a| {
                                        let v: &str = a.value.as_ref();
                                        v.strip_prefix("language-")
                                    })
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                if let Some(tl::enums::MessageEntity::Pre(p)) = entities.last_mut()
                                {
                                    p.language = lang;
                                }
                            } else {
                                entities.push(tl::enums::MessageEntity::Code(
                                    tl::types::MessageEntityCode {
                                        offset,
                                        length: len0,
                                    },
                                ));
                            }
                        }
                        "pre" => entities.push(tl::enums::MessageEntity::Pre(
                            tl::types::MessageEntityPre {
                                offset,
                                length: len0,
                                language: String::new(),
                            },
                        )),
                        "a" => {
                            let href = attrs
                                .iter()
                                .find(|a| a.name.local.as_ref() == "href")
                                .map(|a| {
                                    let v: &str = a.value.as_ref();
                                    v.to_string()
                                })
                                .unwrap_or_default();
                            const MENTION_PFX: &str = "tg://user?id=";
                            if href.starts_with(MENTION_PFX) {
                                if let Ok(uid) = href[MENTION_PFX.len()..].parse::<i64>() {
                                    entities.push(tl::enums::MessageEntity::MentionName(
                                        tl::types::MessageEntityMentionName {
                                            offset,
                                            length: len0,
                                            user_id: uid,
                                        },
                                    ));
                                }
                            } else {
                                entities.push(tl::enums::MessageEntity::TextUrl(
                                    tl::types::MessageEntityTextUrl {
                                        offset,
                                        length: len0,
                                        url: href,
                                    },
                                ));
                            }
                        }
                        "tg-emoji" => {
                            let doc_id = attrs
                                .iter()
                                .find(|a| a.name.local.as_ref() == "emoji-id")
                                .and_then(|a| {
                                    let v: &str = a.value.as_ref();
                                    v.parse::<i64>().ok()
                                })
                                .unwrap_or(0);
                            entities.push(tl::enums::MessageEntity::CustomEmoji(
                                tl::types::MessageEntityCustomEmoji {
                                    offset,
                                    length: len0,
                                    document_id: doc_id,
                                },
                            ));
                        }
                        "br" => {
                            text.push('\n');
                            offset += 1;
                        }
                        _ => {}
                    }
                }
                Token::TagToken(Tag {
                    kind: TagKind::EndTag,
                    name,
                    ..
                }) => match name.as_ref() {
                    "b" | "strong" => close_ent!(Bold),
                    "i" | "em" => close_ent!(Italic),
                    "u" | "ins" => close_ent!(Underline),
                    "s" | "del" | "strike" => close_ent!(Strike),
                    "tg-spoiler" | "span" => close_ent!(Spoiler),
                    "blockquote" => close_ent!(Blockquote),
                    "tg-time" => close_ent!(FormattedDate),
                    "code" => {
                        let in_pre = entities.last().map_or(
                            false,
                            |e| matches!(e, tl::enums::MessageEntity::Pre(p) if p.length == 0),
                        );
                        if !in_pre {
                            close_ent!(Code);
                        }
                    }
                    "pre" => close_ent!(Pre),
                    "a" => match entities.last() {
                        Some(tl::enums::MessageEntity::MentionName(_)) => close_ent!(MentionName),
                        _ => close_ent!(TextUrl),
                    },
                    "tg-emoji" => close_ent!(CustomEmoji),
                    _ => {}
                },
                Token::CharacterTokens(s) => {
                    let s_str: &str = s.as_ref();
                    offset += s_str.encode_utf16().count() as i32;
                    text.push_str(s_str);
                }
                _ => {}
            }

            self.text.replace(text);
            self.entities.replace(entities);
            self.offset.replace(offset);
            TokenSinkResult::Continue
        }
    }

    let mut input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html).try_reinterpret().unwrap());
    let tok = Tokenizer::new(
        Sink {
            text: Cell::new(String::with_capacity(html.len())),
            entities: Cell::new(Vec::new()),
            offset: Cell::new(0),
        },
        Default::default(),
    );
    let _ = tok.feed(&mut input);
    tok.end();
    let Sink { text, entities, .. } = tok.sink;
    (text.take(), entities.take())
}

// HTML generator (always available, no html5ever dependency)

/// Generate Telegram-compatible HTML from plain text + entities.
///
/// All entity types are handled including `Blockquote`, `FormattedDate`
/// (`<tg-time>`), and `CustomEmoji` (`<tg-emoji>`).
pub fn generate_html(text: &str, entities: &[tl::enums::MessageEntity]) -> String {
    use tl::enums::MessageEntity as ME;

    let mut markers: Vec<(i32, bool, String)> = Vec::new();

    for ent in entities {
        let (off, len, open, close) = match ent {
            ME::Bold(e) => (e.offset, e.length, "<b>".into(), "</b>".into()),
            ME::Italic(e) => (e.offset, e.length, "<i>".into(), "</i>".into()),
            ME::Underline(e) => (e.offset, e.length, "<u>".into(), "</u>".into()),
            ME::Strike(e) => (e.offset, e.length, "<s>".into(), "</s>".into()),
            ME::Spoiler(e) => (
                e.offset,
                e.length,
                "<tg-spoiler>".into(),
                "</tg-spoiler>".into(),
            ),
            ME::Code(e) => (e.offset, e.length, "<code>".into(), "</code>".into()),
            ME::Pre(e) => {
                let lang = if e.language.is_empty() {
                    String::new()
                } else {
                    format!(" class=\"language-{}\"", e.language)
                };
                (
                    e.offset,
                    e.length,
                    format!("<pre><code{lang}>"),
                    "</code></pre>".into(),
                )
            }
            ME::TextUrl(e) => (
                e.offset,
                e.length,
                format!("<a href=\"{}\">", escape_html(&e.url)),
                "</a>".into(),
            ),
            ME::MentionName(e) => (
                e.offset,
                e.length,
                format!("<a href=\"tg://user?id={}\">", e.user_id),
                "</a>".into(),
            ),
            ME::CustomEmoji(e) => (
                e.offset,
                e.length,
                format!("<tg-emoji emoji-id=\"{}\">", e.document_id),
                "</tg-emoji>".into(),
            ),
            ME::Blockquote(e) => {
                let open = if e.collapsed {
                    "<blockquote expandable>".to_string()
                } else {
                    "<blockquote>".to_string()
                };
                (e.offset, e.length, open, "</blockquote>".into())
            }
            ME::FormattedDate(e) => {
                let fmt = tg_time_flags_to_format(e);
                (
                    e.offset,
                    e.length,
                    format!("<tg-time unix=\"{}\" format=\"{}\">", e.date, fmt),
                    "</tg-time>".into(),
                )
            }
            _ => continue,
        };
        markers.push((off, true, open));
        markers.push((off + len, false, close));
    }

    markers.sort_by(|(a_pos, a_open, _), (b_pos, b_open, _)| {
        a_pos.cmp(b_pos).then_with(|| b_open.cmp(a_open))
    });

    let mut result =
        String::with_capacity(text.len() + markers.iter().map(|(_, _, s)| s.len()).sum::<usize>());
    let mut marker_idx = 0;
    let mut utf16_pos: i32 = 0;

    for ch in text.chars() {
        while marker_idx < markers.len() && markers[marker_idx].0 <= utf16_pos {
            result.push_str(&markers[marker_idx].2);
            marker_idx += 1;
        }
        match ch {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            c => result.push(c),
        }
        utf16_pos += ch.len_utf16() as i32;
    }
    while marker_idx < markers.len() {
        result.push_str(&markers[marker_idx].2);
        marker_idx += 1;
    }

    result
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
