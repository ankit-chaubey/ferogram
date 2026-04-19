// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram-parsers: Telegram text entity parsers for ferogram
// https://github.com/ankit-chaubey/ferogram

//! Text formatting parsers: HTML and Markdown ↔ Telegram [`MessageEntity`]
//!
//! # Markdown (Telegram-flavoured)
//! ## Parsing (`parse_markdown`)
//! | Syntax | Entity |
//! |--------|--------|
//! | `**bold**` or `*bold*` | Bold |
//! | `__italic__` or `_italic_` | Italic |
//! | `~~strike~~` | Strikethrough |
//! | `\|\|spoiler\|\|` | Spoiler |
//! | `` `code` `` | Code |
//! | ` ```lang\npre\n``` ` | Pre (code block) |
//! | `[text](url)` | TextUrl |
//! | `[text](tg://user?id=123)` | MentionName |
//! | `![text](tg://emoji?id=123)` | CustomEmoji |
//! | `\*`, `\_`, `\~` ... | Escaped literal char |
//!
//! ## Generating (`generate_markdown`)
//! Produces the same syntax above for all supported entity types.
//! `Underline` has no unambiguous markdown delimiter and is silently skipped.
//!
//! # HTML
//! Supported tags: `<b>`, `<strong>`, `<i>`, `<em>`, `<u>`, `<s>`, `<del>`,
//! `<code>`, `<pre>`, `<tg-spoiler>`, `<a href="url">`,
//! `<tg-emoji emoji-id="id">text</tg-emoji>`
//!
//! # Feature gates
//! * `html`     : enables `parse_html` / `generate_html` via the built-in hand-rolled
//!   parser (zero extra deps).
//! * `html5ever`: replaces `parse_html` with a spec-compliant html5ever tokenizer.
//!   `generate_html` is always the same hand-rolled generator.

use ferogram_tl_types as tl;

// Markdown

/// Parse Telegram-flavoured markdown into (plain_text, entities).
pub fn parse_markdown(text: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    let mut out = String::with_capacity(text.len());
    let mut ents = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut open_stack: Vec<(MarkdownTag, i32)> = Vec::new();
    let mut utf16_off: i32 = 0;

    macro_rules! push_char {
        ($c:expr) => {{
            let c: char = $c;
            out.push(c);
            utf16_off += c.len_utf16() as i32;
        }};
    }

    while i < n {
        // backslash escape: \X → literal X (for any special char)
        if chars[i] == '\\' && i + 1 < n {
            let next = chars[i + 1];
            if matches!(
                next,
                '*' | '_' | '~' | '|' | '[' | ']' | '(' | ')' | '`' | '\\' | '!'
            ) {
                push_char!(next);
                i += 2;
                continue;
            }
        }

        // code block: ```lang\n...```
        if i + 2 < n && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            let start = i + 3;
            let mut j = start;
            while j + 2 < n {
                if chars[j] == '`' && chars[j + 1] == '`' && chars[j + 2] == '`' {
                    break;
                }
                j += 1;
            }
            if j + 2 < n {
                let block: String = chars[start..j].iter().collect();
                let (lang, code) = if let Some(nl) = block.find('\n') {
                    (block[..nl].trim().to_string(), block[nl + 1..].to_string())
                } else {
                    (String::new(), block)
                };
                let code_off = utf16_off;
                let code_utf16: i32 = code.encode_utf16().count() as i32;
                ents.push(tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
                    offset: code_off,
                    length: code_utf16,
                    language: lang,
                }));
                for c in code.chars() {
                    push_char!(c);
                }
                i = j + 3;
                continue;
            }
        }

        // inline code: `code`
        if chars[i] == '`' {
            let start = i + 1;
            let mut j = start;
            while j < n && chars[j] != '`' {
                j += 1;
            }
            if j < n {
                let code: String = chars[start..j].iter().collect();
                let code_off = utf16_off;
                let code_utf16: i32 = code.encode_utf16().count() as i32;
                ents.push(tl::enums::MessageEntity::Code(
                    tl::types::MessageEntityCode {
                        offset: code_off,
                        length: code_utf16,
                    },
                ));
                for c in code.chars() {
                    push_char!(c);
                }
                i = j + 1;
                continue;
            }
        }

        // custom emoji: ![text](tg://emoji?id=12345)
        if chars[i] == '!' && i + 1 < n && chars[i + 1] == '[' {
            let text_start = i + 2;
            let mut j = text_start;
            while j < n && chars[j] != ']' {
                j += 1;
            }
            if j < n && j + 1 < n && chars[j + 1] == '(' {
                let link_start = j + 2;
                let mut k = link_start;
                while k < n && chars[k] != ')' {
                    k += 1;
                }
                if k < n {
                    let inner_text: String = chars[text_start..j].iter().collect();
                    let url: String = chars[link_start..k].iter().collect();
                    const EMOJI_PFX: &str = "tg://emoji?id=";
                    if let Some(stripped) = url.strip_prefix(EMOJI_PFX)
                        && let Ok(doc_id) = stripped.parse::<i64>()
                    {
                        let ent_off = utf16_off;
                        for c in inner_text.chars() {
                            push_char!(c);
                        }
                        ents.push(tl::enums::MessageEntity::CustomEmoji(
                            tl::types::MessageEntityCustomEmoji {
                                offset: ent_off,
                                length: utf16_off - ent_off,
                                document_id: doc_id,
                            },
                        ));
                        i = k + 1;
                        continue;
                    }
                }
            }
        }

        // inline link / mention: [text](url) or [text](tg://user?id=123)
        if chars[i] == '[' {
            let text_start = i + 1;
            let mut j = text_start;
            let mut depth = 1i32;
            while j < n {
                if chars[j] == '[' {
                    depth += 1;
                }
                if chars[j] == ']' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if j < n && j + 1 < n && chars[j + 1] == '(' {
                let link_start = j + 2;
                let mut k = link_start;
                while k < n && chars[k] != ')' {
                    k += 1;
                }
                if k < n {
                    let inner_text: String = chars[text_start..j].iter().collect();
                    let url: String = chars[link_start..k].iter().collect();
                    const MENTION_PFX: &str = "tg://user?id=";
                    let ent_off = utf16_off;
                    for c in inner_text.chars() {
                        push_char!(c);
                    }
                    let ent_len = utf16_off - ent_off;
                    if let Some(stripped) = url.strip_prefix(MENTION_PFX) {
                        if let Ok(uid) = stripped.parse::<i64>() {
                            ents.push(tl::enums::MessageEntity::MentionName(
                                tl::types::MessageEntityMentionName {
                                    offset: ent_off,
                                    length: ent_len,
                                    user_id: uid,
                                },
                            ));
                        }
                    } else {
                        ents.push(tl::enums::MessageEntity::TextUrl(
                            tl::types::MessageEntityTextUrl {
                                offset: ent_off,
                                length: ent_len,
                                url,
                            },
                        ));
                    }
                    i = k + 1;
                    continue;
                }
            }
        }

        // two-char delimiters: **, __, ~~, ||
        let two: Option<MarkdownTag> = if i + 1 < n {
            match [chars[i], chars[i + 1]] {
                ['*', '*'] => Some(MarkdownTag::Bold),
                ['_', '_'] => Some(MarkdownTag::Italic),
                ['~', '~'] => Some(MarkdownTag::Strike),
                ['|', '|'] => Some(MarkdownTag::Spoiler),
                _ => None,
            }
        } else {
            None
        };

        if let Some(tag) = two {
            if let Some(pos) = open_stack.iter().rposition(|(t, _)| *t == tag) {
                let (_, start_off) = open_stack.remove(pos);
                let length = utf16_off - start_off;
                if length > 0 {
                    ents.push(make_entity(tag, start_off, length));
                }
            } else {
                open_stack.push((tag, utf16_off));
            }
            i += 2;
            continue;
        }

        // single-char delimiters: *bold*, _italic_
        // Only fires when the current char is NOT part of a two-char sequence.
        let one: Option<MarkdownTag> = match chars[i] {
            '*' => Some(MarkdownTag::Bold),
            '_' => Some(MarkdownTag::Italic),
            _ => None,
        };

        if let Some(tag) = one {
            if let Some(pos) = open_stack.iter().rposition(|(t, _)| *t == tag) {
                let (_, start_off) = open_stack.remove(pos);
                let length = utf16_off - start_off;
                if length > 0 {
                    ents.push(make_entity(tag, start_off, length));
                }
            } else {
                open_stack.push((tag, utf16_off));
            }
            i += 1;
            continue;
        }

        push_char!(chars[i]);
        i += 1;
    }

    (out, ents)
}

fn make_entity(tag: MarkdownTag, offset: i32, length: i32) -> tl::enums::MessageEntity {
    match tag {
        MarkdownTag::Bold => {
            tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold { offset, length })
        }
        MarkdownTag::Italic => {
            tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic { offset, length })
        }
        MarkdownTag::Strike => {
            tl::enums::MessageEntity::Strike(tl::types::MessageEntityStrike { offset, length })
        }
        MarkdownTag::Spoiler => {
            tl::enums::MessageEntity::Spoiler(tl::types::MessageEntitySpoiler { offset, length })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkdownTag {
    Bold,
    Italic,
    Strike,
    Spoiler,
}

/// Generate Telegram markdown from plain text + entities.
///
/// All entity types are handled. `Underline` has no unambiguous markdown
/// delimiter and is silently skipped (use `generate_html` if you need it).
pub fn generate_markdown(text: &str, entities: &[tl::enums::MessageEntity]) -> String {
    use tl::enums::MessageEntity as ME;

    // Each entry is (utf16_position, is_open, marker_string).
    // Pre blocks need a trailing newline before the closing ```.
    let mut insertions: Vec<(i32, bool, String)> = Vec::new();

    for ent in entities {
        match ent {
            ME::Bold(e) => {
                insertions.push((e.offset, true, "**".into()));
                insertions.push((e.offset + e.length, false, "**".into()));
            }
            ME::Italic(e) => {
                insertions.push((e.offset, true, "__".into()));
                insertions.push((e.offset + e.length, false, "__".into()));
            }
            ME::Strike(e) => {
                insertions.push((e.offset, true, "~~".into()));
                insertions.push((e.offset + e.length, false, "~~".into()));
            }
            ME::Spoiler(e) => {
                insertions.push((e.offset, true, "||".into()));
                insertions.push((e.offset + e.length, false, "||".into()));
            }
            ME::Code(e) => {
                insertions.push((e.offset, true, "`".into()));
                insertions.push((e.offset + e.length, false, "`".into()));
            }
            ME::Pre(e) => {
                let lang = e.language.trim();
                insertions.push((e.offset, true, format!("```{lang}\n")));
                insertions.push((e.offset + e.length, false, "\n```".into()));
            }
            ME::TextUrl(e) => {
                insertions.push((e.offset, true, "[".into()));
                insertions.push((e.offset + e.length, false, format!("]({})", e.url)));
            }
            ME::MentionName(e) => {
                insertions.push((e.offset, true, "[".into()));
                insertions.push((
                    e.offset + e.length,
                    false,
                    format!("](tg://user?id={})", e.user_id),
                ));
            }
            ME::CustomEmoji(e) => {
                insertions.push((e.offset, true, "![".into()));
                insertions.push((
                    e.offset + e.length,
                    false,
                    format!("](tg://emoji?id={})", e.document_id),
                ));
            }
            // Underline has no clean markdown delimiter; skip it.
            _ => {}
        }
    }

    // Sort: by position, opens before closes at the same position.
    insertions.sort_by(|(a_pos, a_open, _), (b_pos, b_open, _)| {
        a_pos.cmp(b_pos).then_with(|| b_open.cmp(a_open))
    });

    let mut result = String::with_capacity(
        text.len() + insertions.iter().map(|(_, _, s)| s.len()).sum::<usize>(),
    );
    let mut ins_idx = 0;
    let mut utf16_pos: i32 = 0;

    for ch in text.chars() {
        while ins_idx < insertions.len() && insertions[ins_idx].0 <= utf16_pos {
            result.push_str(&insertions[ins_idx].2);
            ins_idx += 1;
        }
        // Escape markdown special chars in plain text.
        match ch {
            '*' | '_' | '~' | '|' | '[' | ']' | '(' | ')' | '`' | '\\' | '!' => {
                result.push('\\');
                result.push(ch);
            }
            c => result.push(c),
        }
        utf16_pos += ch.len_utf16() as i32;
    }
    while ins_idx < insertions.len() {
        result.push_str(&insertions[ins_idx].2);
        ins_idx += 1;
    }

    result
}

// HTML parser: built-in hand-rolled (no extra deps)
// Compiled when `html5ever` feature is NOT active.

/// Parse a Telegram-compatible HTML string into (plain_text, entities).
///
/// Hand-rolled, zero-dependency implementation.  Override with the
/// `html5ever` Cargo feature for a spec-compliant tokenizer.
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
                if let Some(pos) = stack.iter().rposition(|(t, _, _)| t.name() == tag_name) {
                    let (htag, start_off, extra) = stack.remove(pos);
                    let length = utf16_off - start_off;
                    if length > 0 {
                        let entity = match htag {
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
                            HtmlTag::Spoiler => Some(tl::enums::MessageEntity::Spoiler(
                                tl::types::MessageEntitySpoiler {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::Code => Some(tl::enums::MessageEntity::Code(
                                tl::types::MessageEntityCode {
                                    offset: start_off,
                                    length,
                                },
                            )),
                            HtmlTag::Pre => {
                                Some(tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
                                    offset: start_off,
                                    length,
                                    language: extra.unwrap_or_default(),
                                }))
                            }
                            HtmlTag::Link(url) => {
                                const PFX: &str = "tg://user?id=";
                                if let Some(stripped) = url.strip_prefix(PFX) {
                                    stripped.parse::<i64>().ok().map(|uid| {
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
                            HtmlTag::Unknown => None,
                        };
                        if let Some(e) = entity {
                            ents.push(e);
                        }
                    }
                }
            } else {
                let htag = match tag_name {
                    "b" | "strong" => HtmlTag::Bold,
                    "i" | "em" => HtmlTag::Italic,
                    "u" => HtmlTag::Underline,
                    "s" | "del" | "strike" => HtmlTag::Strike,
                    "tg-spoiler" => HtmlTag::Spoiler,
                    "code" => HtmlTag::Code,
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
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{00A0}")
}

#[cfg(not(feature = "html5ever"))]
fn parse_tag(s: &str) -> (&str, Vec<(String, String)>) {
    let mut parts = s.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").trim_end_matches('/');
    let attrs = parse_attrs(parts.next().unwrap_or(""));
    (name, attrs)
}

#[cfg(not(feature = "html5ever"))]
fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut rem = s.trim();
    while !rem.is_empty() {
        if let Some(eq) = rem.find('=') {
            let key = rem[..eq].trim().to_string();
            rem = rem[eq + 1..].trim_start();
            let (val, rest) = if let Some(s) = rem.strip_prefix('"') {
                let end = s.find('"').map(|p| p + 1).unwrap_or(rem.len() - 1);
                (rem[1..end].to_string(), &rem[end + 1..])
            } else if let Some(s) = rem.strip_prefix('\'') {
                let end = s.find('\'').map(|p| p + 1).unwrap_or(rem.len() - 1);
                (rem[1..end].to_string(), &rem[end + 1..])
            } else {
                let end = rem.find(char::is_whitespace).unwrap_or(rem.len());
                (rem[..end].to_string(), &rem[end..])
            };
            result.push((key, val));
            rem = rest.trim_start();
        } else {
            break;
        }
    }
    result
}

#[cfg(not(feature = "html5ever"))]
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum HtmlTag {
    Bold,
    Italic,
    Underline,
    Strike,
    Spoiler,
    Code,
    Pre,
    Link(String),
    CustomEmoji(i64),
    Unknown,
}

#[cfg(not(feature = "html5ever"))]
impl HtmlTag {
    fn name(&self) -> &str {
        match self {
            Self::Bold => "b",
            Self::Italic => "i",
            Self::Underline => "u",
            Self::Strike => "s",
            Self::Spoiler => "tg-spoiler",
            Self::Code => "code",
            Self::Pre => "pre",
            Self::Link(_) => "a",
            Self::CustomEmoji(_) => "tg-emoji",
            Self::Unknown => "",
        }
    }
}

// HTML parser: html5ever backend
// Compiled when `html5ever` feature IS active; overrides the built-in parser.

/// Parse a Telegram-compatible HTML string into (plain_text, entities).
///
/// Uses the [`html5ever`] spec-compliant tokenizer.  Enable the `html5ever`
/// Cargo feature to activate this implementation.
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

            // Close the most-recent open entity of `$kind` (open = length==0).
            // Removes the entity if start == end (zero-length element).
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
                // Start tags
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
                        "u" => entities.push(tl::enums::MessageEntity::Underline(
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
                        "code" => {
                            // Inside an open <pre>? Annotate language on the pre entity.
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
                                if let Some(tl::enums::MessageEntity::Pre(ref mut p)) =
                                    entities.last_mut()
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

                // End tags
                Token::TagToken(Tag {
                    kind: TagKind::EndTag,
                    name,
                    ..
                }) => {
                    match name.as_ref() {
                        "b" | "strong" => close_ent!(Bold),
                        "i" | "em" => close_ent!(Italic),
                        "u" => close_ent!(Underline),
                        "s" | "del" | "strike" => close_ent!(Strike),
                        "tg-spoiler" => close_ent!(Spoiler),
                        "code" => {
                            // Inside open <pre>: pre absorbs the code tag.
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
                            Some(tl::enums::MessageEntity::MentionName(_)) => {
                                close_ent!(MentionName)
                            }
                            _ => close_ent!(TextUrl),
                        },
                        "tg-emoji" => close_ent!(CustomEmoji),
                        _ => {}
                    }
                }

                // Text content
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

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_bold() {
        let (text, ents) = parse_markdown("Hello **world**!");
        assert_eq!(text, "Hello world!");
        assert_eq!(ents.len(), 1);
        if let tl::enums::MessageEntity::Bold(b) = &ents[0] {
            assert_eq!(b.offset, 6);
            assert_eq!(b.length, 5);
        } else {
            panic!("expected bold");
        }
    }

    #[test]
    fn markdown_bold_single_asterisk() {
        let (text, ents) = parse_markdown("*bold*");
        assert_eq!(text, "bold");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Bold(_)));
    }

    #[test]
    fn markdown_italic_double_underscore() {
        let (text, ents) = parse_markdown("__italic__");
        assert_eq!(text, "italic");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Italic(_)));
    }

    #[test]
    fn markdown_italic_single_underscore() {
        let (text, ents) = parse_markdown("_italic_");
        assert_eq!(text, "italic");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Italic(_)));
    }

    #[test]
    fn markdown_inline_code() {
        let (text, ents) = parse_markdown("Use `foo()` to do it");
        assert_eq!(text, "Use foo() to do it");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Code(_)));
    }

    #[test]
    fn markdown_code_block_with_lang() {
        let (text, ents) = parse_markdown("```rust\nfn main() {}\n```");
        assert_eq!(text, "fn main() {}");
        if let tl::enums::MessageEntity::Pre(p) = &ents[0] {
            assert_eq!(p.language, "rust");
            assert_eq!(p.offset, 0);
        } else {
            panic!("expected pre");
        }
    }

    #[test]
    fn markdown_code_block_no_lang() {
        let (text, ents) = parse_markdown("```\nhello\n```");
        assert_eq!(text, "hello");
        if let tl::enums::MessageEntity::Pre(p) = &ents[0] {
            assert_eq!(p.language, "");
        } else {
            panic!("expected pre");
        }
    }

    #[test]
    fn markdown_strike() {
        let (text, ents) = parse_markdown("~~strike~~");
        assert_eq!(text, "strike");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Strike(_)));
    }

    #[test]
    fn markdown_spoiler() {
        let (text, ents) = parse_markdown("||spoiler||");
        assert_eq!(text, "spoiler");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Spoiler(_)));
    }

    #[test]
    fn markdown_text_url() {
        let (text, ents) = parse_markdown("[click](https://example.com)");
        assert_eq!(text, "click");
        if let tl::enums::MessageEntity::TextUrl(e) = &ents[0] {
            assert_eq!(e.url, "https://example.com");
        } else {
            panic!("expected text url");
        }
    }

    #[test]
    fn markdown_mention() {
        let (text, ents) = parse_markdown("[User](tg://user?id=42)");
        assert_eq!(text, "User");
        if let tl::enums::MessageEntity::MentionName(e) = &ents[0] {
            assert_eq!(e.user_id, 42);
        } else {
            panic!("expected mention name");
        }
    }

    #[test]
    fn markdown_custom_emoji() {
        let (text, ents) = parse_markdown("![👍](tg://emoji?id=5368324170671202286)");
        assert_eq!(text, "👍");
        if let tl::enums::MessageEntity::CustomEmoji(e) = &ents[0] {
            assert_eq!(e.document_id, 5368324170671202286);
        } else {
            panic!("expected custom emoji");
        }
    }

    #[test]
    fn markdown_backslash_escape() {
        let (text, ents) = parse_markdown(r"\*not bold\*");
        assert_eq!(text, "*not bold*");
        assert!(ents.is_empty());
    }

    #[test]
    fn markdown_nested() {
        let (text, ents) = parse_markdown("**bold __italic__ end**");
        assert_eq!(text, "bold italic end");
        assert_eq!(ents.len(), 2);
        assert!(
            ents.iter()
                .any(|e| matches!(e, tl::enums::MessageEntity::Bold(_)))
        );
        assert!(
            ents.iter()
                .any(|e| matches!(e, tl::enums::MessageEntity::Italic(_)))
        );
    }

    #[test]
    fn generate_markdown_pre() {
        let entities = vec![tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
            offset: 0,
            length: 12,
            language: "rust".into(),
        })];
        let md = generate_markdown("fn main() {}", &entities);
        assert_eq!(md, "```rust\nfn main() {}\n```");
    }

    #[test]
    fn generate_markdown_text_url() {
        let entities = vec![tl::enums::MessageEntity::TextUrl(
            tl::types::MessageEntityTextUrl {
                offset: 0,
                length: 5,
                url: "https://example.com".into(),
            },
        )];
        let md = generate_markdown("click", &entities);
        assert_eq!(md, "[click](https://example.com)");
    }

    #[test]
    fn generate_markdown_mention() {
        let entities = vec![tl::enums::MessageEntity::MentionName(
            tl::types::MessageEntityMentionName {
                offset: 0,
                length: 4,
                user_id: 99,
            },
        )];
        let md = generate_markdown("User", &entities);
        assert_eq!(md, "[User](tg://user?id=99)");
    }

    #[test]
    fn generate_markdown_custom_emoji() {
        let entities = vec![tl::enums::MessageEntity::CustomEmoji(
            tl::types::MessageEntityCustomEmoji {
                offset: 0,
                length: 2,
                document_id: 123456,
            },
        )];
        let md = generate_markdown("👍", &entities);
        assert_eq!(md, "![👍](tg://emoji?id=123456)");
    }

    #[test]
    fn generate_markdown_escapes_special_chars() {
        let (_, empty): (_, Vec<_>) = (String::new(), vec![]);
        let md = generate_markdown("1 * 2 = 2", &empty);
        assert_eq!(md, r"1 \* 2 = 2");
    }

    #[test]
    fn markdown_roundtrip_url() {
        let original = "click";
        let entities = vec![tl::enums::MessageEntity::TextUrl(
            tl::types::MessageEntityTextUrl {
                offset: 0,
                length: 5,
                url: "https://example.com".into(),
            },
        )];
        let md = generate_markdown(original, &entities);
        let (back, ents2) = parse_markdown(&md);
        assert_eq!(back, original);
        if let tl::enums::MessageEntity::TextUrl(e) = &ents2[0] {
            assert_eq!(e.url, "https://example.com");
        } else {
            panic!("roundtrip url failed");
        }
    }

    #[test]
    fn html_bold_italic() {
        let (text, ents) = parse_html("<b>bold</b> and <i>italic</i>");
        assert_eq!(text, "bold and italic");
        assert_eq!(ents.len(), 2);
    }

    #[test]
    fn html_link() {
        let (text, ents) = parse_html("<a href=\"https://example.com\">click</a>");
        assert_eq!(text, "click");
        if let tl::enums::MessageEntity::TextUrl(e) = &ents[0] {
            assert_eq!(e.url, "https://example.com");
        } else {
            panic!("expected text url");
        }
    }

    // HTML entity decoding is a hand-rolled-only feature; html5ever handles it natively.
    #[cfg(not(feature = "html5ever"))]
    #[test]
    fn html_entities_decoded() {
        let (text, _) = parse_html("A &amp; B &lt;3&gt;");
        assert_eq!(text, "A & B <3>");
    }

    #[test]
    fn generate_html_roundtrip() {
        let original = "Hello world";
        let entities = vec![tl::enums::MessageEntity::Bold(
            tl::types::MessageEntityBold {
                offset: 0,
                length: 5,
            },
        )];
        let html = generate_html(original, &entities);
        assert_eq!(html, "<b>Hello</b> world");
        let (back, ents2) = parse_html(&html);
        assert_eq!(back, original);
        assert_eq!(ents2.len(), 1);
    }
}
