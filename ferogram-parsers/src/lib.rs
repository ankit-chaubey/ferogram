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

#![deny(unsafe_code)]

use ferogram_tl_types as tl;

// Markdown V1 (legacy, retained for backward compatibility)

/// Parse Telegram-flavoured **MarkdownV1** (legacy) into `(plain_text, entities)`.
///
/// This is the original Telegram Bot API Markdown format.  It is retained for
/// libraries and bots that still target the legacy format, but **should not be
/// used in new code**.  Prefer [`parse_markdown`] (V2) instead.
///
/// Notable V1 limitations vs V2:
/// - `__text__` produces *Italic* (not Underline).
/// - `~~text~~` produces Strike (double-tilde, GitHub-style, not Telegram spec).
/// - No Blockquote, Expandable Blockquote support.
/// - Smaller set of backslash-escapable characters.
#[deprecated(
    since = "0.3.9",
    note = "Telegram considers MarkdownV1 legacy. Use `parse_markdown` (V2) for new code."
)]
pub fn parse_markdown_v1(text: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    parse_markdown_v1_impl(text)
}

fn parse_markdown_v1_impl(text: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    let mut out = String::with_capacity(text.len());
    let mut ents = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut open_stack: Vec<(MarkdownTagV1, i32)> = Vec::new();
    let mut utf16_off: i32 = 0;

    macro_rules! push_char {
        ($c:expr) => {{
            let c: char = $c;
            out.push(c);
            utf16_off += c.len_utf16() as i32;
        }};
    }

    while i < n {
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

        // Code block: ```lang\n...```
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
                    (
                        block[..nl].trim().to_string(),
                        block[nl + 1..].trim_end_matches('\n').to_string(),
                    )
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

        // Inline code: `code`
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

        // Custom emoji: ![text](tg://emoji?id=N)
        if chars[i] == '!'
            && i + 1 < n
            && chars[i + 1] == '['
            && let Some((end_i, doc_id, inner_text)) = parse_emoji_link(&chars, i)
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
            i = end_i;
            continue;
        }

        // Inline link/mention: [text](url)
        if chars[i] == '['
            && let Some((end_i, ent)) =
                parse_link_entity(&chars, i, utf16_off, &mut out, &mut utf16_off)
        {
            ents.push(ent);
            i = end_i;
            continue;
        }

        // Two-char delimiters: **, __, ~~, ||
        if i + 1 < n {
            let tag = match [chars[i], chars[i + 1]] {
                ['*', '*'] => Some(MarkdownTagV1::Bold),
                ['_', '_'] => Some(MarkdownTagV1::Italic), // V1: __ = italic
                ['~', '~'] => Some(MarkdownTagV1::Strike), // V1: ~~ = strike (GitHub-style)
                ['|', '|'] => Some(MarkdownTagV1::Spoiler),
                _ => None,
            };
            if let Some(tag) = tag {
                toggle_tag_v1(&mut open_stack, &mut ents, tag, utf16_off);
                i += 2;
                continue;
            }
        }

        // Single-char: *, _
        let one_tag = match chars[i] {
            '*' => Some(MarkdownTagV1::Bold),
            '_' => Some(MarkdownTagV1::Italic),
            _ => None,
        };
        if let Some(tag) = one_tag {
            toggle_tag_v1(&mut open_stack, &mut ents, tag, utf16_off);
            i += 1;
            continue;
        }

        push_char!(chars[i]);
        i += 1;
    }

    (out, ents)
}

fn toggle_tag_v1(
    stack: &mut Vec<(MarkdownTagV1, i32)>,
    ents: &mut Vec<tl::enums::MessageEntity>,
    tag: MarkdownTagV1,
    utf16_off: i32,
) {
    if let Some(pos) = stack.iter().rposition(|(t, _)| *t == tag) {
        let (_, start_off) = stack.remove(pos);
        let length = utf16_off - start_off;
        if length > 0 {
            ents.push(make_entity_v1(tag, start_off, length));
        }
    } else {
        stack.push((tag, utf16_off));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkdownTagV1 {
    Bold,
    Italic,
    Strike,
    Spoiler,
}

fn make_entity_v1(tag: MarkdownTagV1, offset: i32, length: i32) -> tl::enums::MessageEntity {
    match tag {
        MarkdownTagV1::Bold => {
            tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold { offset, length })
        }
        MarkdownTagV1::Italic => {
            tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic { offset, length })
        }
        MarkdownTagV1::Strike => {
            tl::enums::MessageEntity::Strike(tl::types::MessageEntityStrike { offset, length })
        }
        MarkdownTagV1::Spoiler => {
            tl::enums::MessageEntity::Spoiler(tl::types::MessageEntitySpoiler { offset, length })
        }
    }
}

// Markdown V2 (current Telegram Bot API format)

/// Parse **MarkdownV2** (current Telegram format) into `(plain_text, entities)`.
///
/// Full Telegram Bot API MarkdownV2 specification:
///
/// | Syntax | Entity |
/// |---|---|
/// | `*text*` or `**text**` | Bold |
/// | `_text_` | Italic |
/// | `__text__` | **Underline** |
/// | `~text~` | Strikethrough |
/// | `\|\|text\|\|` | Spoiler |
/// | `` `code` `` | Inline code |
/// | ```` ```lang\nblock\n``` ```` | Code block |
/// | `[label](url)` | Text URL |
/// | `[label](tg://user?id=N)` | Mention by ID |
/// | `![label](tg://emoji?id=N)` | Custom emoji (empty label OK) |
/// | `>line` at line start | Blockquote |
/// | `**>line` at line start | Expandable (collapsed) blockquote |
///
/// Backslash escapes: `_ * [ ] ( ) ~ \\ \` > # + - = | { } . !`
///
/// This is also callable as [`parse_markdown`].
pub fn parse_markdown_v2(text: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    let mut out = String::with_capacity(text.len());
    let mut ents = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut open_stack: Vec<(MarkdownTagV2, i32)> = Vec::new();
    let mut utf16_off: i32 = 0;

    // Blockquote state
    let mut bq_start_off: Option<i32> = None;
    let mut bq_collapsed = false;
    let mut at_line_start = true;

    macro_rules! push_char {
        ($c:expr) => {{
            let c: char = $c;
            out.push(c);
            utf16_off += c.len_utf16() as i32;
        }};
    }

    while i < n {
        // Blockquote detection at line start
        if at_line_start {
            let is_exp = i + 2 < n && chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '>';
            let is_bq = !is_exp && chars[i] == '>';

            if is_exp || is_bq {
                let collapsed = is_exp;
                match bq_start_off {
                    None => {
                        bq_start_off = Some(utf16_off);
                        bq_collapsed = collapsed;
                    }
                    Some(start_off) if bq_collapsed != collapsed => {
                        // Close old, open new type
                        let length = utf16_off - start_off;
                        if length > 0 {
                            ents.push(make_blockquote(start_off, length, bq_collapsed));
                        }
                        bq_start_off = Some(utf16_off);
                        bq_collapsed = collapsed;
                    }
                    _ => {} // same type: keep accumulating
                }
                i += if is_exp { 3 } else { 1 };
                if i < n && chars[i] == ' ' {
                    i += 1;
                } // skip optional space
                at_line_start = false;
                continue;
            } else if let Some(start_off) = bq_start_off.take() {
                // Normal line after blockquote: close it
                let length = utf16_off - start_off;
                if length > 0 {
                    ents.push(make_blockquote(start_off, length, bq_collapsed));
                }
            }
            at_line_start = false;
        }

        // Backslash escape (V2 set)
        if chars[i] == '\\' && i + 1 < n && is_v2_escapable(chars[i + 1]) {
            push_char!(chars[i + 1]);
            i += 2;
            continue;
        }

        // Code block: ```lang\n...```
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
                    (
                        block[..nl].trim().to_string(),
                        block[nl + 1..].trim_end_matches('\n').to_string(),
                    )
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

        // Inline code: `code`
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

        // Custom emoji: ![label](tg://emoji?id=N) (empty label OK)
        if chars[i] == '!'
            && i + 1 < n
            && chars[i + 1] == '['
            && let Some((end_i, doc_id, inner_text)) = parse_emoji_link(&chars, i)
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
            i = end_i;
            continue;
        }

        // Inline link/mention: [text](url)
        if chars[i] == '['
            && let Some((end_i, ent)) =
                parse_link_entity(&chars, i, utf16_off, &mut out, &mut utf16_off)
        {
            ents.push(ent);
            i = end_i;
            continue;
        }

        // Two-char delimiters: **, __, ||
        if i + 1 < n {
            let tag = match [chars[i], chars[i + 1]] {
                ['*', '*'] => Some(MarkdownTagV2::Bold),
                ['_', '_'] => Some(MarkdownTagV2::Underline), // V2: __ = Underline
                ['|', '|'] => Some(MarkdownTagV2::Spoiler),
                _ => None,
            };
            if let Some(tag) = tag {
                toggle_tag_v2(&mut open_stack, &mut ents, tag, utf16_off);
                i += 2;
                continue;
            }
        }

        // Single-char: *, _, ~
        let one_tag = match chars[i] {
            '*' => Some(MarkdownTagV2::Bold),
            '_' => Some(MarkdownTagV2::Italic),
            '~' => Some(MarkdownTagV2::Strike), // V2: single ~ = strike
            _ => None,
        };
        if let Some(tag) = one_tag {
            toggle_tag_v2(&mut open_stack, &mut ents, tag, utf16_off);
            i += 1;
            continue;
        }

        // Newline (tracks line-start for blockquote)
        if chars[i] == '\n' {
            push_char!('\n');
            at_line_start = true;
            i += 1;
            continue;
        }

        push_char!(chars[i]);
        i += 1;
    }

    // Close any unclosed blockquote at end of input
    if let Some(start_off) = bq_start_off.take() {
        let length = utf16_off - start_off;
        if length > 0 {
            ents.push(make_blockquote(start_off, length, bq_collapsed));
        }
    }

    (out, ents)
}

fn toggle_tag_v2(
    stack: &mut Vec<(MarkdownTagV2, i32)>,
    ents: &mut Vec<tl::enums::MessageEntity>,
    tag: MarkdownTagV2,
    utf16_off: i32,
) {
    if let Some(pos) = stack.iter().rposition(|(t, _)| *t == tag) {
        let (_, start_off) = stack.remove(pos);
        let length = utf16_off - start_off;
        if length > 0 {
            ents.push(make_entity_v2(tag, start_off, length));
        }
    } else {
        stack.push((tag, utf16_off));
    }
}

fn is_v2_escapable(c: char) -> bool {
    matches!(
        c,
        '_' | '*'
            | '['
            | ']'
            | '('
            | ')'
            | '~'
            | '\\'
            | '`'
            | '>'
            | '#'
            | '+'
            | '-'
            | '='
            | '|'
            | '{'
            | '}'
            | '.'
            | '!'
    )
}

fn make_blockquote(offset: i32, length: i32, collapsed: bool) -> tl::enums::MessageEntity {
    tl::enums::MessageEntity::Blockquote(tl::types::MessageEntityBlockquote {
        collapsed,
        offset,
        length,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkdownTagV2 {
    Bold,
    Italic,
    Underline,
    Strike,
    Spoiler,
}

fn make_entity_v2(tag: MarkdownTagV2, offset: i32, length: i32) -> tl::enums::MessageEntity {
    match tag {
        MarkdownTagV2::Bold => {
            tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold { offset, length })
        }
        MarkdownTagV2::Italic => {
            tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic { offset, length })
        }
        MarkdownTagV2::Underline => {
            tl::enums::MessageEntity::Underline(tl::types::MessageEntityUnderline {
                offset,
                length,
            })
        }
        MarkdownTagV2::Strike => {
            tl::enums::MessageEntity::Strike(tl::types::MessageEntityStrike { offset, length })
        }
        MarkdownTagV2::Spoiler => {
            tl::enums::MessageEntity::Spoiler(tl::types::MessageEntitySpoiler { offset, length })
        }
    }
}

// parse_markdown / generate_markdown: public entry points (default = V2)

/// Parse Telegram markdown into `(plain_text, entities)`.
///
/// Uses **MarkdownV2** (current Telegram Bot API format)
/// See [`parse_markdown_v2`] for full syntax reference.
///
/// For the legacy V1 behaviour call [`parse_markdown_v1`] explicitly.
pub fn parse_markdown(text: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    parse_markdown_v2(text)
}

/// Generate **MarkdownV2** from plain text + entities.
///
/// | Entity | Markdown |
/// |---|---|
/// | Bold | `*text*` |
/// | Italic | `_text_` |
/// | Underline | `__text__` |
/// | Strike | `~text~` |
/// | Spoiler | `\|\|text\|\|` |
/// | Blockquote | `>` line prefix |
/// | Expandable blockquote | `**>` line prefix |
/// | Code | `` `text` `` |
/// | Pre | ```` ```lang\ncode\n``` ```` |
/// | TextUrl | `[text](url)` |
/// | MentionName | `[text](tg://user?id=N)` |
/// | CustomEmoji | `![text](tg://emoji?id=N)` |
///
/// All V2 special characters in plain-text spans are backslash-escaped.
pub fn generate_markdown_v2(text: &str, entities: &[tl::enums::MessageEntity]) -> String {
    use tl::enums::MessageEntity as ME;

    struct BqRange {
        offset: i32,
        end: i32,
        collapsed: bool,
    }
    let mut bq_ranges: Vec<BqRange> = Vec::new();
    let mut pre_ranges: Vec<(i32, i32)> = Vec::new();
    let mut code_ranges: Vec<(i32, i32)> = Vec::new();
    let mut insertions: Vec<(i32, bool, String)> = Vec::new();

    for ent in entities {
        match ent {
            ME::Bold(e) => {
                insertions.push((e.offset, true, "*".into()));
                insertions.push((e.offset + e.length, false, "*".into()));
            }
            ME::Italic(e) => {
                insertions.push((e.offset, true, "_".into()));
                insertions.push((e.offset + e.length, false, "_".into()));
            }
            ME::Underline(e) => {
                insertions.push((e.offset, true, "__".into()));
                insertions.push((e.offset + e.length, false, "__".into()));
            }
            ME::Strike(e) => {
                insertions.push((e.offset, true, "~".into()));
                insertions.push((e.offset + e.length, false, "~".into()));
            }
            ME::Spoiler(e) => {
                insertions.push((e.offset, true, "||".into()));
                insertions.push((e.offset + e.length, false, "||".into()));
            }
            ME::Code(e) => {
                insertions.push((e.offset, true, "`".into()));
                insertions.push((e.offset + e.length, false, "`".into()));
                code_ranges.push((e.offset, e.offset + e.length));
            }
            ME::Pre(e) => {
                let lang = e.language.trim();
                insertions.push((e.offset, true, format!("```{lang}\n")));
                insertions.push((e.offset + e.length, false, "\n```".into()));
                pre_ranges.push((e.offset, e.offset + e.length));
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
            ME::Blockquote(e) => {
                bq_ranges.push(BqRange {
                    offset: e.offset,
                    end: e.offset + e.length,
                    collapsed: e.collapsed,
                });
            }
            _ => {}
        }
    }

    insertions.sort_by(|(a_pos, a_open, _), (b_pos, b_open, _)| {
        a_pos.cmp(b_pos).then_with(|| b_open.cmp(a_open))
    });

    let mut result = String::with_capacity(text.len() + 64);
    let mut ins_idx = 0;
    let mut utf16_pos: i32 = 0;
    let mut at_line_start = true;

    for ch in text.chars() {
        while ins_idx < insertions.len() && insertions[ins_idx].0 <= utf16_pos {
            result.push_str(&insertions[ins_idx].2);
            ins_idx += 1;
        }

        // Emit blockquote line prefix
        if at_line_start
            && let Some(bq) = bq_ranges
                .iter()
                .find(|b| utf16_pos >= b.offset && utf16_pos < b.end)
        {
            if bq.collapsed {
                result.push_str("**>");
            } else {
                result.push('>');
            }
            result.push(' ');
        }

        let in_verbatim = pre_ranges
            .iter()
            .any(|(s, e)| utf16_pos >= *s && utf16_pos < *e)
            || code_ranges
                .iter()
                .any(|(s, e)| utf16_pos >= *s && utf16_pos < *e);

        if !in_verbatim && is_v2_escapable(ch) {
            result.push('\\');
        }
        result.push(ch);

        utf16_pos += ch.len_utf16() as i32;
        at_line_start = ch == '\n';
    }
    while ins_idx < insertions.len() {
        result.push_str(&insertions[ins_idx].2);
        ins_idx += 1;
    }

    result
}

/// Generate Telegram markdown from plain text + entities.
/// Calls [`generate_markdown_v2`].
pub fn generate_markdown(text: &str, entities: &[tl::enums::MessageEntity]) -> String {
    generate_markdown_v2(text, entities)
}

// Shared parse helpers

/// Parse `![label](tg://emoji?id=N)` at `start` (position of `!`).
/// Returns `(end_idx, document_id, inner_text)` or `None`.
/// Empty `inner_text` is valid (MarkdownV2).
fn parse_emoji_link(chars: &[char], start: usize) -> Option<(usize, i64, String)> {
    let n = chars.len();
    let text_start = start + 2; // skip `![`
    let mut j = text_start;
    while j < n && chars[j] != ']' {
        j += 1;
    }
    if j >= n || j + 1 >= n || chars[j + 1] != '(' {
        return None;
    }
    let link_start = j + 2;
    let mut k = link_start;
    while k < n && chars[k] != ')' {
        k += 1;
    }
    if k >= n {
        return None;
    }
    let inner_text: String = chars[text_start..j].iter().collect();
    let url: String = chars[link_start..k].iter().collect();
    let doc_id = url.strip_prefix("tg://emoji?id=")?.parse::<i64>().ok()?;
    Some((k + 1, doc_id, inner_text))
}

/// Parse `[text](url)` at `start` (position of `[`).
/// Mutates `out` and `utf16_off` to emit the inner text.
/// Returns `(end_idx, entity)` or `None`.
fn parse_link_entity(
    chars: &[char],
    start: usize,
    utf16_off_in: i32,
    out: &mut String,
    utf16_off: &mut i32,
) -> Option<(usize, tl::enums::MessageEntity)> {
    let n = chars.len();
    let text_start = start + 1;
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
    if j >= n || j + 1 >= n || chars[j + 1] != '(' {
        return None;
    }
    let link_start = j + 2;
    let mut k = link_start;
    while k < n && chars[k] != ')' {
        k += 1;
    }
    if k >= n {
        return None;
    }

    let inner_text: String = chars[text_start..j].iter().collect();
    let url: String = chars[link_start..k].iter().collect();
    let ent_off = utf16_off_in;
    for c in inner_text.chars() {
        out.push(c);
        *utf16_off += c.len_utf16() as i32;
    }
    let ent_len = *utf16_off - ent_off;

    const MENTION_PFX: &str = "tg://user?id=";
    let ent = if let Some(id_str) = url.strip_prefix(MENTION_PFX) {
        if let Ok(uid) = id_str.parse::<i64>() {
            tl::enums::MessageEntity::MentionName(tl::types::MessageEntityMentionName {
                offset: ent_off,
                length: ent_len,
                user_id: uid,
            })
        } else {
            tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
                offset: ent_off,
                length: ent_len,
                url,
            })
        }
    } else {
        tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
            offset: ent_off,
            length: ent_len,
            url,
        })
    };
    Some((k + 1, ent))
}

// tg-time format helpers

/// Map `<tg-time format="…">` string to individual flag booleans.
///
/// Format characters (combinable): `r`/`R`=relative, `t`=short-time,
/// `T`/`tt`=long-time, `d`=short-date, `D`=long-date, `w`/`W`=day-of-week.
fn parse_tg_time_format(fmt: &str) -> (bool, bool, bool, bool, bool, bool) {
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

/// Map `MessageEntityFormattedDate` flags back to a format string.
fn tg_time_flags_to_format(e: &tl::types::MessageEntityFormattedDate) -> String {
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

// HTML parser: hand-rolled (no extra deps)

/// Parse a Telegram-compatible HTML string into `(plain_text, entities)`.
///
/// Hand-rolled, zero-dependency implementation.  Enable the `html5ever`
/// Cargo feature for a spec-compliant tokenizer.
///
/// **Supported tags:**
/// `<b>`, `<strong>`, `<i>`, `<em>`, `<u>`, `<ins>`, `<s>`, `<del>`,
/// `<strike>`, `<tg-spoiler>`, `<span class="tg-spoiler">`,
/// `<a href="…">`, `<tg-emoji emoji-id="N">`, `<code>`,
/// `<pre>`, `<pre><code class="language-X">`,
/// `<blockquote>`, `<blockquote expandable>`,
/// `<tg-time unix="N" format="F">`, `<br>`.
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

/// Parse HTML attributes including boolean attributes (e.g. `expandable`).
#[cfg(not(feature = "html5ever"))]
fn parse_attrs(s: &str) -> Vec<(String, String)> {
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

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    // Markdown V2 (default)

    #[test]
    fn markdown_bold() {
        let (text, ents) = parse_markdown("Hello **world**!");
        assert_eq!(text, "Hello world!");
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
    fn markdown_italic_single_underscore() {
        let (text, ents) = parse_markdown("_italic_");
        assert_eq!(text, "italic");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Italic(_)));
    }

    /// V2: `__text__` = Underline (not Italic)
    #[test]
    fn markdown_v2_underline_double_underscore() {
        let (text, ents) = parse_markdown("__underline__");
        assert_eq!(text, "underline");
        assert!(
            matches!(ents[0], tl::enums::MessageEntity::Underline(_)),
            "expected Underline, got {:?}",
            ents[0]
        );
    }

    /// V2: single `~text~` = Strikethrough
    #[test]
    fn markdown_v2_strike_single_tilde() {
        let (text, ents) = parse_markdown("~strike~");
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

    /// V2: empty label in custom emoji is valid
    #[test]
    fn markdown_v2_custom_emoji_empty_label() {
        let (text, ents) = parse_markdown("![](tg://emoji?id=12345)");
        assert_eq!(text, "");
        if let tl::enums::MessageEntity::CustomEmoji(e) = &ents[0] {
            assert_eq!(e.document_id, 12345);
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
    fn markdown_v2_backslash_escape_extended() {
        let (text, ents) = parse_markdown(r"\>\=\.");
        assert_eq!(text, ">=.");
        assert!(ents.is_empty());
    }

    #[test]
    fn markdown_v2_nested_bold_italic() {
        let (text, ents) = parse_markdown("**bold _italic_ end**");
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
    fn markdown_v2_blockquote_single_line() {
        let (text, ents) = parse_markdown("> hello");
        assert_eq!(text, "hello");
        if let tl::enums::MessageEntity::Blockquote(e) = &ents[0] {
            assert!(!e.collapsed);
        } else {
            panic!("expected blockquote, got {:?}", ents[0]);
        }
    }

    #[test]
    fn markdown_v2_blockquote_multi_line() {
        let (text, ents) = parse_markdown("> line1\n> line2");
        assert_eq!(text, "line1\nline2");
        assert_eq!(ents.len(), 1);
        assert!(matches!(ents[0], tl::enums::MessageEntity::Blockquote(_)));
    }

    #[test]
    fn markdown_v2_expandable_blockquote() {
        let (text, ents) = parse_markdown("**> secret");
        assert_eq!(text, "secret");
        if let tl::enums::MessageEntity::Blockquote(e) = &ents[0] {
            assert!(
                e.collapsed,
                "expandable blockquote should have collapsed=true"
            );
        } else {
            panic!("expected blockquote");
        }
    }

    // Markdown V1 (legacy)

    /// V1: `__text__` = Italic (legacy behaviour)
    #[test]
    fn markdown_v1_italic_double_underscore() {
        #[allow(deprecated)]
        let (text, ents) = parse_markdown_v1("__italic__");
        assert_eq!(text, "italic");
        assert!(
            matches!(ents[0], tl::enums::MessageEntity::Italic(_)),
            "V1 __ should be Italic"
        );
    }

    /// V1: `~~text~~` = Strike
    #[test]
    fn markdown_v1_strike_double_tilde() {
        #[allow(deprecated)]
        let (text, ents) = parse_markdown_v1("~~strike~~");
        assert_eq!(text, "strike");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Strike(_)));
    }

    // Markdown V2 generator

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
    fn generate_markdown_v2_escapes_special_chars() {
        let (_, empty): (_, Vec<_>) = (String::new(), vec![]);
        let md = generate_markdown("1 * 2 = 2", &empty);
        assert_eq!(md, r"1 \* 2 \= 2");
    }

    #[test]
    fn generate_markdown_v2_italic_and_underline() {
        let entities = vec![
            tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic {
                offset: 0,
                length: 6,
            }),
            tl::enums::MessageEntity::Underline(tl::types::MessageEntityUnderline {
                offset: 7,
                length: 9,
            }),
        ];
        let md = generate_markdown("italic underline", &entities);
        assert_eq!(md, "_italic_ __underline__");
    }

    #[test]
    fn generate_markdown_v2_strike() {
        let entities = vec![tl::enums::MessageEntity::Strike(
            tl::types::MessageEntityStrike {
                offset: 0,
                length: 6,
            },
        )];
        let md = generate_markdown("struck", &entities);
        assert_eq!(md, "~struck~");
    }

    #[test]
    fn generate_markdown_v2_blockquote() {
        let entities = vec![tl::enums::MessageEntity::Blockquote(
            tl::types::MessageEntityBlockquote {
                collapsed: false,
                offset: 0,
                length: 5,
            },
        )];
        let md = generate_markdown("hello", &entities);
        assert!(md.starts_with("> "), "expected '> ' prefix, got: {md:?}");
        assert!(md.contains("hello"));
    }

    #[test]
    fn generate_markdown_v2_expandable_blockquote() {
        let entities = vec![tl::enums::MessageEntity::Blockquote(
            tl::types::MessageEntityBlockquote {
                collapsed: true,
                offset: 0,
                length: 6,
            },
        )];
        let md = generate_markdown("secret", &entities);
        assert!(
            md.starts_with("**> "),
            "expected '**> ' prefix, got: {md:?}"
        );
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

    // HTML parser

    #[test]
    fn html_bold_italic() {
        let (text, ents) = parse_html("<b>bold</b> and <i>italic</i>");
        assert_eq!(text, "bold and italic");
        assert_eq!(ents.len(), 2);
    }

    #[test]
    fn html_strong_em_aliases() {
        let (text, ents) = parse_html("<strong>bold</strong> <em>italic</em>");
        assert_eq!(text, "bold italic");
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
    fn html_ins_alias_for_underline() {
        let (text, ents) = parse_html("<ins>underline</ins>");
        assert_eq!(text, "underline");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Underline(_)));
    }

    #[test]
    fn html_span_tg_spoiler() {
        let (text, ents) = parse_html("<span class=\"tg-spoiler\">hidden</span>");
        assert_eq!(text, "hidden");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Spoiler(_)));
    }

    #[test]
    fn html_blockquote() {
        let (text, ents) = parse_html("<blockquote>quoted</blockquote>");
        assert_eq!(text, "quoted");
        if let tl::enums::MessageEntity::Blockquote(e) = &ents[0] {
            assert!(!e.collapsed);
        } else {
            panic!("expected blockquote");
        }
    }

    #[test]
    fn html_blockquote_expandable() {
        let (text, ents) = parse_html("<blockquote expandable>secret</blockquote>");
        assert_eq!(text, "secret");
        if let tl::enums::MessageEntity::Blockquote(e) = &ents[0] {
            assert!(
                e.collapsed,
                "expandable blockquote should have collapsed=true"
            );
        } else {
            panic!("expected blockquote");
        }
    }

    #[test]
    fn html_tg_time() {
        let (text, ents) =
            parse_html("<tg-time unix=\"1700000000\" format=\"Dt\">Nov 14</tg-time>");
        assert_eq!(text, "Nov 14");
        if let tl::enums::MessageEntity::FormattedDate(e) = &ents[0] {
            assert_eq!(e.date, 1700000000);
            assert!(e.long_date);
            assert!(e.short_time);
        } else {
            panic!("expected FormattedDate, got {:?}", ents[0]);
        }
    }

    #[test]
    fn html_pre_with_language() {
        let (text, ents) =
            parse_html("<pre><code class=\"language-rust\">fn main() {}</code></pre>");
        assert_eq!(text, "fn main() {}");
        assert_eq!(ents.len(), 1, "should be exactly one Pre entity");
        if let tl::enums::MessageEntity::Pre(p) = &ents[0] {
            assert_eq!(p.language, "rust");
        } else {
            panic!("expected pre");
        }
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

    #[cfg(not(feature = "html5ever"))]
    #[test]
    fn html_entities_decoded() {
        let (text, _) = parse_html("A &amp; B &lt;3&gt;");
        assert_eq!(text, "A & B <3>");
    }

    // HTML generator

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

    #[test]
    fn generate_html_blockquote() {
        let entities = vec![tl::enums::MessageEntity::Blockquote(
            tl::types::MessageEntityBlockquote {
                collapsed: false,
                offset: 0,
                length: 6,
            },
        )];
        let html = generate_html("quoted", &entities);
        assert_eq!(html, "<blockquote>quoted</blockquote>");
    }

    #[test]
    fn generate_html_expandable_blockquote() {
        let entities = vec![tl::enums::MessageEntity::Blockquote(
            tl::types::MessageEntityBlockquote {
                collapsed: true,
                offset: 0,
                length: 6,
            },
        )];
        let html = generate_html("secret", &entities);
        assert_eq!(html, "<blockquote expandable>secret</blockquote>");
    }

    #[test]
    fn generate_html_formatted_date() {
        let entities = vec![tl::enums::MessageEntity::FormattedDate(
            tl::types::MessageEntityFormattedDate {
                relative: false,
                short_time: true,
                long_time: false,
                short_date: false,
                long_date: true,
                day_of_week: false,
                offset: 0,
                length: 6,
                date: 1700000000,
            },
        )];
        let html = generate_html("Nov 14", &entities);
        assert!(html.contains("tg-time"), "expected tg-time in: {html}");
        assert!(html.contains("1700000000"));
    }

    #[test]
    fn html_pre_with_language_roundtrip() {
        let original = "fn main() {}";
        let entities = vec![tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
            offset: 0,
            length: 12,
            language: "rust".into(),
        })];
        let html = generate_html(original, &entities);
        let (back, ents2) = parse_html(&html);
        assert_eq!(back, original);
        if let tl::enums::MessageEntity::Pre(p) = &ents2[0] {
            assert_eq!(p.language, "rust");
        } else {
            panic!("roundtrip pre language failed");
        }
    }

    // HTML parse: remaining tag aliases

    #[test]
    fn html_u_underline() {
        let (text, ents) = parse_html("<u>under</u>");
        assert_eq!(text, "under");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Underline(_)));
    }

    #[test]
    fn html_s_strike() {
        let (text, ents) = parse_html("<s>gone</s>");
        assert_eq!(text, "gone");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Strike(_)));
    }

    #[test]
    fn html_del_strike_alias() {
        let (text, ents) = parse_html("<del>gone</del>");
        assert_eq!(text, "gone");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Strike(_)));
    }

    #[test]
    fn html_strike_tag_alias() {
        let (text, ents) = parse_html("<strike>gone</strike>");
        assert_eq!(text, "gone");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Strike(_)));
    }

    #[test]
    fn html_tg_spoiler_tag() {
        let (text, ents) = parse_html("<tg-spoiler>secret</tg-spoiler>");
        assert_eq!(text, "secret");
        assert!(matches!(ents[0], tl::enums::MessageEntity::Spoiler(_)));
    }

    #[test]
    fn html_tg_emoji() {
        let (text, ents) = parse_html("<tg-emoji emoji-id=\"9876\">X</tg-emoji>");
        assert_eq!(text, "X");
        if let tl::enums::MessageEntity::CustomEmoji(e) = &ents[0] {
            assert_eq!(e.document_id, 9876);
            assert_eq!(e.offset, 0);
            assert_eq!(e.length, 1);
        } else {
            panic!("expected CustomEmoji, got {:?}", ents[0]);
        }
    }

    #[test]
    fn html_mention_name() {
        let (text, ents) = parse_html("<a href=\"tg://user?id=777\">Alice</a>");
        assert_eq!(text, "Alice");
        if let tl::enums::MessageEntity::MentionName(e) = &ents[0] {
            assert_eq!(e.user_id, 777);
        } else {
            panic!("expected MentionName, got {:?}", ents[0]);
        }
    }

    #[test]
    fn html_inline_code() {
        let (text, ents) = parse_html("call <code>foo()</code> now");
        assert_eq!(text, "call foo() now");
        if let tl::enums::MessageEntity::Code(e) = &ents[0] {
            assert_eq!(e.offset, 5);
            assert_eq!(e.length, 5);
        } else {
            panic!("expected Code");
        }
    }

    // HTML parse: offset correctness

    #[test]
    fn html_offset_mid_string() {
        // "Hello bold end" -> bold at offset 6, length 4
        let (text, ents) = parse_html("Hello <b>bold</b> end");
        assert_eq!(text, "Hello bold end");
        if let tl::enums::MessageEntity::Bold(e) = &ents[0] {
            assert_eq!(e.offset, 6);
            assert_eq!(e.length, 4);
        } else {
            panic!("expected Bold");
        }
    }

    #[test]
    fn html_nested_bold_italic() {
        let (text, ents) = parse_html("<b>bold <i>both</i> bold</b>");
        assert_eq!(text, "bold both bold");
        assert_eq!(ents.len(), 2);
        let bold = ents
            .iter()
            .find(|e| matches!(e, tl::enums::MessageEntity::Bold(_)))
            .unwrap();
        if let tl::enums::MessageEntity::Bold(e) = bold {
            assert_eq!(e.offset, 0);
            assert_eq!(e.length, 14); // whole string
        }
        let italic = ents
            .iter()
            .find(|e| matches!(e, tl::enums::MessageEntity::Italic(_)))
            .unwrap();
        if let tl::enums::MessageEntity::Italic(e) = italic {
            assert_eq!(e.offset, 5);
            assert_eq!(e.length, 4); // "both"
        }
    }

    // HTML generate: all entity types

    #[test]
    fn generate_html_italic() {
        let entities = vec![tl::enums::MessageEntity::Italic(
            tl::types::MessageEntityItalic {
                offset: 0,
                length: 4,
            },
        )];
        assert_eq!(generate_html("test", &entities), "<i>test</i>");
    }

    #[test]
    fn generate_html_underline() {
        let entities = vec![tl::enums::MessageEntity::Underline(
            tl::types::MessageEntityUnderline {
                offset: 0,
                length: 4,
            },
        )];
        assert_eq!(generate_html("test", &entities), "<u>test</u>");
    }

    #[test]
    fn generate_html_strike() {
        let entities = vec![tl::enums::MessageEntity::Strike(
            tl::types::MessageEntityStrike {
                offset: 0,
                length: 4,
            },
        )];
        assert_eq!(generate_html("test", &entities), "<s>test</s>");
    }

    #[test]
    fn generate_html_spoiler() {
        let entities = vec![tl::enums::MessageEntity::Spoiler(
            tl::types::MessageEntitySpoiler {
                offset: 0,
                length: 6,
            },
        )];
        assert_eq!(
            generate_html("secret", &entities),
            "<tg-spoiler>secret</tg-spoiler>"
        );
    }

    #[test]
    fn generate_html_inline_code() {
        let entities = vec![tl::enums::MessageEntity::Code(
            tl::types::MessageEntityCode {
                offset: 0,
                length: 3,
            },
        )];
        assert_eq!(generate_html("foo", &entities), "<code>foo</code>");
    }

    #[test]
    fn generate_html_pre_no_lang() {
        let entities = vec![tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
            offset: 0,
            length: 4,
            language: String::new(),
        })];
        let html = generate_html("code", &entities);
        assert_eq!(html, "<pre><code>code</code></pre>");
    }

    #[test]
    fn generate_html_custom_emoji() {
        let entities = vec![tl::enums::MessageEntity::CustomEmoji(
            tl::types::MessageEntityCustomEmoji {
                offset: 0,
                length: 2,
                document_id: 555,
            },
        )];
        assert_eq!(
            generate_html("ok", &entities),
            "<tg-emoji emoji-id=\"555\">ok</tg-emoji>"
        );
    }

    #[test]
    fn generate_html_mention_name() {
        let entities = vec![tl::enums::MessageEntity::MentionName(
            tl::types::MessageEntityMentionName {
                offset: 0,
                length: 3,
                user_id: 42,
            },
        )];
        assert_eq!(
            generate_html("Bob", &entities),
            "<a href=\"tg://user?id=42\">Bob</a>"
        );
    }

    // HTML generate: escaping in plain text

    #[test]
    fn generate_html_escapes_special_chars() {
        let (_, empty): (_, Vec<_>) = (String::new(), vec![]);
        let html = generate_html("a & b < c > d \"e\"", &empty);
        assert_eq!(html, "a &amp; b &lt; c &gt; d &quot;e&quot;");
    }

    // UTF-16 offset correctness (multibyte chars)

    #[test]
    fn utf16_offset_emoji_before_entity() {
        // "👍 bold": emoji is 2 UTF-16 code units, bold starts at offset 3 (2 + space)
        let (text, ents) = parse_markdown("👍 **bold**");
        assert_eq!(text, "👍 bold");
        if let tl::enums::MessageEntity::Bold(e) = &ents[0] {
            assert_eq!(e.offset, 3); // 2 (emoji) + 1 (space)
            assert_eq!(e.length, 4);
        } else {
            panic!("expected Bold");
        }
    }

    #[test]
    fn utf16_offset_emoji_bold_in_html() {
        let (text, ents) = parse_html("👍 <b>bold</b>");
        assert_eq!(text, "👍 bold");
        if let tl::enums::MessageEntity::Bold(e) = &ents[0] {
            assert_eq!(e.offset, 3);
            assert_eq!(e.length, 4);
        } else {
            panic!("expected Bold");
        }
    }

    #[test]
    fn utf16_offset_cjk_char() {
        // CJK char is 1 UTF-16 unit (U+4E2D = 20013), bold starts at offset 1
        let (text, ents) = parse_markdown("中**bold**");
        assert_eq!(text, "中bold");
        if let tl::enums::MessageEntity::Bold(e) = &ents[0] {
            assert_eq!(e.offset, 1);
            assert_eq!(e.length, 4);
        } else {
            panic!("expected Bold");
        }
    }

    #[test]
    fn utf16_offset_surrogate_pair_inside_entity() {
        // Bold text that itself contains an emoji: "👍" is 2 UTF-16 units
        let (text, ents) = parse_markdown("**👍**");
        assert_eq!(text, "👍");
        if let tl::enums::MessageEntity::Bold(e) = &ents[0] {
            assert_eq!(e.offset, 0);
            assert_eq!(e.length, 2); // emoji = 2 UTF-16 units
        } else {
            panic!("expected Bold");
        }
    }

    // Markdown generate: missing types

    #[test]
    fn generate_markdown_v2_spoiler() {
        let entities = vec![tl::enums::MessageEntity::Spoiler(
            tl::types::MessageEntitySpoiler {
                offset: 0,
                length: 6,
            },
        )];
        assert_eq!(generate_markdown("secret", &entities), "||secret||");
    }

    #[test]
    fn generate_markdown_v2_bold() {
        let entities = vec![tl::enums::MessageEntity::Bold(
            tl::types::MessageEntityBold {
                offset: 0,
                length: 4,
            },
        )];
        assert_eq!(generate_markdown("bold", &entities), "*bold*");
    }

    // Markdown roundtrip: all inline types

    fn roundtrip_md(text: &str, ent: tl::enums::MessageEntity) {
        let md = generate_markdown(text, &[ent]);
        let (back, ents2) = parse_markdown(&md);
        assert_eq!(back, text, "roundtrip text mismatch for: {md:?}");
        assert_eq!(ents2.len(), 1, "roundtrip entity count wrong for: {md:?}");
    }

    #[test]
    fn markdown_roundtrip_bold() {
        roundtrip_md(
            "x",
            tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn markdown_roundtrip_italic() {
        roundtrip_md(
            "x",
            tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn markdown_roundtrip_underline() {
        roundtrip_md(
            "x",
            tl::enums::MessageEntity::Underline(tl::types::MessageEntityUnderline {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn markdown_roundtrip_strike() {
        roundtrip_md(
            "x",
            tl::enums::MessageEntity::Strike(tl::types::MessageEntityStrike {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn markdown_roundtrip_spoiler() {
        roundtrip_md(
            "x",
            tl::enums::MessageEntity::Spoiler(tl::types::MessageEntitySpoiler {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn markdown_roundtrip_code() {
        roundtrip_md(
            "x",
            tl::enums::MessageEntity::Code(tl::types::MessageEntityCode {
                offset: 0,
                length: 1,
            }),
        );
    }

    // HTML roundtrip: all entity types

    fn roundtrip_html(text: &str, ent: tl::enums::MessageEntity) {
        let html = generate_html(text, &[ent]);
        let (back, ents2) = parse_html(&html);
        assert_eq!(back, text, "roundtrip text mismatch for: {html:?}");
        assert_eq!(ents2.len(), 1, "roundtrip entity count wrong for: {html:?}");
    }

    #[test]
    fn html_roundtrip_bold() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_italic() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_underline() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Underline(tl::types::MessageEntityUnderline {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_strike() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Strike(tl::types::MessageEntityStrike {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_spoiler() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Spoiler(tl::types::MessageEntitySpoiler {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_code() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Code(tl::types::MessageEntityCode {
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_blockquote() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Blockquote(tl::types::MessageEntityBlockquote {
                collapsed: false,
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_blockquote_expandable() {
        roundtrip_html(
            "x",
            tl::enums::MessageEntity::Blockquote(tl::types::MessageEntityBlockquote {
                collapsed: true,
                offset: 0,
                length: 1,
            }),
        );
    }
    #[test]
    fn html_roundtrip_custom_emoji() {
        let html = generate_html(
            "ok",
            &[tl::enums::MessageEntity::CustomEmoji(
                tl::types::MessageEntityCustomEmoji {
                    offset: 0,
                    length: 2,
                    document_id: 999,
                },
            )],
        );
        let (back, ents2) = parse_html(&html);
        assert_eq!(back, "ok");
        if let tl::enums::MessageEntity::CustomEmoji(e) = &ents2[0] {
            assert_eq!(e.document_id, 999);
        } else {
            panic!("expected CustomEmoji");
        }
    }
    #[test]
    fn html_roundtrip_mention_name() {
        let html = generate_html(
            "Bob",
            &[tl::enums::MessageEntity::MentionName(
                tl::types::MessageEntityMentionName {
                    offset: 0,
                    length: 3,
                    user_id: 42,
                },
            )],
        );
        let (back, ents2) = parse_html(&html);
        assert_eq!(back, "Bob");
        if let tl::enums::MessageEntity::MentionName(e) = &ents2[0] {
            assert_eq!(e.user_id, 42);
        } else {
            panic!("expected MentionName");
        }
    }
}
