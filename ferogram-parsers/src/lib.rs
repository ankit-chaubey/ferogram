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

fn rt_empty() -> tl::enums::RichText {
    tl::enums::RichText::TextEmpty
}

fn rt_plain(s: impl Into<String>) -> tl::enums::RichText {
    let t = s.into();
    if t.is_empty() {
        return rt_empty();
    }
    tl::enums::RichText::TextPlain(tl::types::TextPlain { text: t })
}

fn rt_bold(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextBold(Box::new(tl::types::TextBold { text: inner }))
}

fn rt_italic(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextItalic(Box::new(tl::types::TextItalic { text: inner }))
}

fn rt_underline(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextUnderline(Box::new(tl::types::TextUnderline { text: inner }))
}

fn rt_strike(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextStrike(Box::new(tl::types::TextStrike { text: inner }))
}

fn rt_fixed(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextFixed(Box::new(tl::types::TextFixed { text: inner }))
}

fn rt_marked(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextMarked(Box::new(tl::types::TextMarked { text: inner }))
}

fn rt_spoiler(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextSpoiler(Box::new(tl::types::TextSpoiler { text: inner }))
}

fn rt_subscript(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextSubscript(Box::new(tl::types::TextSubscript { text: inner }))
}

fn rt_superscript(inner: tl::enums::RichText) -> tl::enums::RichText {
    tl::enums::RichText::TextSuperscript(Box::new(tl::types::TextSuperscript { text: inner }))
}

fn rt_url(inner: tl::enums::RichText, url: String) -> tl::enums::RichText {
    tl::enums::RichText::TextUrl(Box::new(tl::types::TextUrl {
        text: inner,
        url,
        webpage_id: 0,
    }))
}

fn rt_email(inner: tl::enums::RichText, email: String) -> tl::enums::RichText {
    tl::enums::RichText::TextEmail(Box::new(tl::types::TextEmail { text: inner, email }))
}

fn rt_phone(inner: tl::enums::RichText, phone: String) -> tl::enums::RichText {
    tl::enums::RichText::TextPhone(Box::new(tl::types::TextPhone { text: inner, phone }))
}

fn rt_mention_name(inner: tl::enums::RichText, user_id: i64) -> tl::enums::RichText {
    tl::enums::RichText::TextMentionName(Box::new(tl::types::TextMentionName {
        text: inner,
        user_id,
    }))
}

fn rt_custom_emoji(document_id: i64, alt: String) -> tl::enums::RichText {
    tl::enums::RichText::TextCustomEmoji(tl::types::TextCustomEmoji { document_id, alt })
}

fn rt_math(source: String) -> tl::enums::RichText {
    tl::enums::RichText::TextMath(tl::types::TextMath { source })
}

fn rt_anchor(inner: tl::enums::RichText, name: String) -> tl::enums::RichText {
    tl::enums::RichText::TextAnchor(Box::new(tl::types::TextAnchor { text: inner, name }))
}

fn rt_date(inner: tl::enums::RichText, date: i32, fmt: &str) -> tl::enums::RichText {
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

fn rt_concat(parts: Vec<tl::enums::RichText>) -> tl::enums::RichText {
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

fn empty_caption() -> tl::enums::PageCaption {
    tl::enums::PageCaption::PageCaption(tl::types::PageCaption {
        text: rt_empty(),
        credit: rt_empty(),
    })
}

fn caption_text(text: tl::enums::RichText) -> tl::enums::PageCaption {
    tl::enums::PageCaption::PageCaption(tl::types::PageCaption {
        text,
        credit: rt_empty(),
    })
}

fn caption_text_credit(
    text: tl::enums::RichText,
    credit: tl::enums::RichText,
) -> tl::enums::PageCaption {
    tl::enums::PageCaption::PageCaption(tl::types::PageCaption { text, credit })
}

// Determine media type from URL (extension/mime heuristic)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaKind {
    Photo,
    Video,
    Audio,
    Voice,
    Animation,
}

fn media_kind_from_url(url: &str) -> MediaKind {
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
fn media_block(url: &str, caption: tl::enums::PageCaption, spoiler: bool) -> tl::enums::PageBlock {
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

fn parse_rich_inline_md_chars(
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
fn try_parse_md_link(chars: &[char], start: usize, end: usize) -> Option<(usize, String, String)> {
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

fn strip_url_title(s: &str) -> String {
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

fn find_two_char_close(chars: &[char], from: usize, end: usize, ch: char) -> Option<usize> {
    let mut i = from;
    while i + 1 < end {
        if chars[i] == ch && chars[i + 1] == ch {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_one_char_close(chars: &[char], from: usize, end: usize, ch: char) -> Option<usize> {
    let mut i = from;
    while i < end {
        if chars[i] == ch {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn build_link_rt(inner: tl::enums::RichText, url: &str, label: &str) -> tl::enums::RichText {
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

fn parse_tg_scheme(
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
fn try_parse_html_inline_tag(
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

// Rich Markdown parser: `&str` → `Vec<PageBlock>`

/// Parse **Rich Markdown** into a list of `PageBlock`s for use in `InputRichMessage`.
///
/// Supports headings H1-H6, paragraphs, code blocks (with language), dividers,
/// unordered/ordered/task lists, blockquotes, media blocks, collage/slideshow,
/// tables, details/summary, footnotes, math blocks, and all inline formatting.
pub fn parse_rich_markdown(text: &str) -> Vec<tl::enums::PageBlock> {
    RichMdParser::new(text).parse()
}

struct RichMdParser<'a> {
    lines: Vec<&'a str>,
    pos: usize,
}

impl<'a> RichMdParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lines: text.lines().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<&str> {
        self.lines.get(self.pos).copied()
    }

    fn parse(mut self) -> Vec<tl::enums::PageBlock> {
        let mut blocks = Vec::new();
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];

            // Skip footnote definition lines (already pre-parsed)
            if line.starts_with("[^") && line.contains("]:") {
                self.pos += 1;
                continue;
            }

            // Empty line
            if line.trim().is_empty() {
                self.pos += 1;
                continue;
            }

            // Fenced code block: ```lang
            if line.trim_start().starts_with("```") {
                if let Some(b) = self.parse_code_block() {
                    blocks.push(b);
                }
                continue;
            }

            // Math block: $$…$$ as block (single line) or ```math
            if line.trim() == "$$"
                || (line.starts_with("$$")
                    && !line.trim_end().ends_with("$$")
                    && line.trim_start() == line)
            {
                // Multi-line $$…$$ block
                if let Some(b) = self.parse_math_block_dollar() {
                    blocks.push(b);
                    continue;
                }
            }
            if line.trim().starts_with("$$") && line.trim().ends_with("$$") && line.trim().len() > 4
            {
                let src = line.trim()[2..line.trim().len() - 2].trim().to_string();
                blocks.push(tl::enums::PageBlock::Math(tl::types::PageBlockMath {
                    source: src,
                }));
                self.pos += 1;
                continue;
            }

            // Divider: ---  or ***  or ___
            if matches!(
                line.trim(),
                "---" | "***" | "___" | "- - -" | "* * *" | "_ _ _"
            ) {
                blocks.push(tl::enums::PageBlock::Divider);
                self.pos += 1;
                continue;
            }

            // Headings: # H1 … ###### H6
            if line.starts_with('#') {
                let level = line.chars().take_while(|&c| c == '#').count();
                if level <= 6 {
                    let rest = line[level..].trim_start();
                    // Strip trailing `#` from ATX headings
                    let heading_text = rest.trim_end_matches('#').trim();
                    let rt = parse_rich_inline_md(heading_text);
                    blocks.push(heading_block(level, rt));
                    self.pos += 1;
                    continue;
                }
            }

            // Blockquote: >
            if line.starts_with('>') {
                blocks.extend(self.parse_blockquote());
                continue;
            }

            // HTML block tags: <details>, <tg-collage>, <tg-slideshow>
            let trimmed = line.trim();
            if trimmed.starts_with('<')
                && let Some(block_result) = self.try_parse_html_block()
            {
                blocks.extend(block_result);
                continue;
            }

            // Unordered list: - / * / +
            if list_unordered_start(line) {
                blocks.push(self.parse_unordered_list());
                continue;
            }

            // Ordered list: 1. / 1)
            if list_ordered_start(line) {
                blocks.push(self.parse_ordered_list());
                continue;
            }

            // Table: | col | col |
            if line.trim_start().starts_with('|')
                && let Some(b) = self.parse_table()
            {
                blocks.push(b);
                continue;
            }

            // Inline media block: ![](url) or ![](url "caption")
            if (trimmed.starts_with("![](") || trimmed.starts_with("![](http"))
                && let Some(b) = try_parse_media_line(trimmed)
            {
                blocks.push(b);
                self.pos += 1;
                continue;
            }

            // Paragraph (default)
            blocks.push(self.parse_paragraph());
        }
        blocks
    }

    fn parse_code_block(&mut self) -> Option<tl::enums::PageBlock> {
        let open = self.lines[self.pos].trim_start();
        let lang = open.strip_prefix("```")?.trim();
        let is_math = lang == "math";
        let lang = lang.to_string();
        self.pos += 1;
        let mut code_lines: Vec<&str> = Vec::new();
        loop {
            match self.lines.get(self.pos).copied() {
                None => break,
                Some(l) if l.trim() == "```" => {
                    self.pos += 1;
                    break;
                }
                Some(l) => {
                    code_lines.push(l);
                    self.pos += 1;
                }
            }
        }
        let code = code_lines.join("\n");
        if is_math {
            Some(tl::enums::PageBlock::Math(tl::types::PageBlockMath {
                source: code,
            }))
        } else {
            Some(tl::enums::PageBlock::Preformatted(
                tl::types::PageBlockPreformatted {
                    text: rt_plain(code),
                    language: lang,
                },
            ))
        }
    }

    fn parse_math_block_dollar(&mut self) -> Option<tl::enums::PageBlock> {
        self.pos += 1; // skip opening $$
        let mut src_lines: Vec<&str> = Vec::new();
        loop {
            match self.lines.get(self.pos).copied() {
                None => break,
                Some(l) if l.trim() == "$$" => {
                    self.pos += 1;
                    break;
                }
                Some(l) => {
                    src_lines.push(l);
                    self.pos += 1;
                }
            }
        }
        Some(tl::enums::PageBlock::Math(tl::types::PageBlockMath {
            source: src_lines.join("\n"),
        }))
    }

    fn parse_blockquote(&mut self) -> Vec<tl::enums::PageBlock> {
        let mut bq_lines = Vec::new();
        while let Some(l) = self.peek() {
            if l.starts_with('>') {
                let content = l
                    .strip_prefix('>')
                    .unwrap_or("")
                    .strip_prefix(' ')
                    .unwrap_or(l.strip_prefix('>').unwrap_or(""));
                bq_lines.push(content.to_string());
                self.pos += 1;
            } else {
                break;
            }
        }
        let combined = bq_lines.join("\n");
        let rt = parse_rich_inline_md(&combined);
        vec![tl::enums::PageBlock::Blockquote(
            tl::types::PageBlockBlockquote {
                text: rt,
                caption: rt_empty(),
            },
        )]
    }

    fn parse_unordered_list(&mut self) -> tl::enums::PageBlock {
        let mut items: Vec<tl::enums::PageListItem> = Vec::new();
        while let Some(line) = self.lines.get(self.pos).copied() {
            if let Some((checked, text)) = parse_list_item_unordered(line) {
                let text = text.to_string();
                let checked_val = checked;
                self.pos += 1;
                let rt = parse_rich_inline_md(&text);
                items.push(tl::enums::PageListItem::Text(tl::types::PageListItemText {
                    checkbox: checked_val.is_some(),
                    checked: checked_val.unwrap_or(false),
                    text: rt,
                }));
            } else {
                break;
            }
        }
        tl::enums::PageBlock::List(tl::types::PageBlockList { items })
    }

    fn parse_ordered_list(&mut self) -> tl::enums::PageBlock {
        let mut items: Vec<tl::enums::PageListOrderedItem> = Vec::new();
        let mut start_num: Option<i32> = None;
        while let Some(line) = self.lines.get(self.pos).copied() {
            if let Some((num, text)) = parse_list_item_ordered(line) {
                let text = text.to_string();
                let num_val = num;
                if start_num.is_none() {
                    start_num = Some(num_val);
                }
                self.pos += 1;
                let rt = parse_rich_inline_md(&text);
                items.push(tl::enums::PageListOrderedItem::Text(
                    tl::types::PageListOrderedItemText {
                        checkbox: false,
                        checked: false,
                        num: None,
                        text: rt,
                        value: Some(num_val),
                        r#type: None,
                    },
                ));
            } else {
                break;
            }
        }
        tl::enums::PageBlock::OrderedList(tl::types::PageBlockOrderedList {
            reversed: false,
            items,
            start: start_num,
            r#type: None,
        })
    }

    fn parse_table(&mut self) -> Option<tl::enums::PageBlock> {
        let header_line = self.lines[self.pos];
        let headers: Vec<&str> = split_table_row(header_line);
        if headers.is_empty() {
            return None;
        }
        self.pos += 1;

        // Separator line
        let mut aligns: Vec<u8> = Vec::new(); // 0=left, 1=center, 2=right
        if let Some(sep) = self.peek()
            && sep.trim_start().starts_with('|')
            && sep.contains('-')
        {
            let cols = split_table_row(sep);
            for col in &cols {
                let c = col.trim();
                let center = c.starts_with(':') && c.ends_with(':');
                let right = !c.starts_with(':') && c.ends_with(':');
                aligns.push(if center {
                    1
                } else if right {
                    2
                } else {
                    0
                });
            }
            self.pos += 1;
        }

        // Header row
        let header_cells: Vec<tl::enums::PageTableCell> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let align_center = aligns.get(i).copied() == Some(1);
                let align_right = aligns.get(i).copied() == Some(2);
                tl::enums::PageTableCell::PageTableCell(tl::types::PageTableCell {
                    header: true,
                    align_center,
                    align_right,
                    valign_middle: false,
                    valign_bottom: false,
                    text: Some(parse_rich_inline_md(h.trim())),
                    colspan: None,
                    rowspan: None,
                })
            })
            .collect();
        let mut rows = vec![tl::enums::PageTableRow::PageTableRow(
            tl::types::PageTableRow {
                cells: header_cells,
            },
        )];

        // Data rows
        while let Some(line) = self.peek() {
            if !line.trim_start().starts_with('|') {
                break;
            }
            let cells_raw = split_table_row(line);
            let cells: Vec<tl::enums::PageTableCell> = cells_raw
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let align_center = aligns.get(i).copied() == Some(1);
                    let align_right = aligns.get(i).copied() == Some(2);
                    tl::enums::PageTableCell::PageTableCell(tl::types::PageTableCell {
                        header: false,
                        align_center,
                        align_right,
                        valign_middle: false,
                        valign_bottom: false,
                        text: Some(parse_rich_inline_md(c.trim())),
                        colspan: None,
                        rowspan: None,
                    })
                })
                .collect();
            rows.push(tl::enums::PageTableRow::PageTableRow(
                tl::types::PageTableRow { cells },
            ));
            self.pos += 1;
        }

        Some(tl::enums::PageBlock::Table(tl::types::PageBlockTable {
            bordered: false,
            striped: false,
            title: rt_empty(),
            rows,
        }))
    }

    fn parse_paragraph(&mut self) -> tl::enums::PageBlock {
        let mut para_lines: Vec<String> = Vec::new();
        while let Some(line) = self.lines.get(self.pos).copied() {
            if line.trim().is_empty() {
                break;
            }
            if line.starts_with('#')
                || line.starts_with('>')
                || list_unordered_start(line)
                || list_ordered_start(line)
                || line.trim_start().starts_with('|')
                || matches!(line.trim(), "---" | "***" | "___")
                || line.trim_start().starts_with("```")
                || (line.trim_start().starts_with('<') && is_block_html_tag(line.trim()))
            {
                break;
            }
            para_lines.push(line.to_string());
            self.pos += 1;
        }
        let combined = para_lines.join("\n");
        let rt = parse_rich_inline_md(&combined);
        tl::enums::PageBlock::Paragraph(tl::types::PageBlockParagraph { text: rt })
    }

    fn try_parse_html_block(&mut self) -> Option<Vec<tl::enums::PageBlock>> {
        let line = self.lines[self.pos];
        let trimmed = line.trim();

        // <details ...><summary>...</summary>
        if trimmed.to_ascii_lowercase().starts_with("<details") {
            return Some(self.parse_details_block());
        }

        // <tg-collage>
        if trimmed.to_ascii_lowercase().starts_with("<tg-collage") {
            return Some(vec![self.parse_collage_block("tg-collage")]);
        }

        // <tg-slideshow>
        if trimmed.to_ascii_lowercase().starts_with("<tg-slideshow") {
            return Some(vec![self.parse_collage_block("tg-slideshow")]);
        }

        // <aside>text<cite>credit</cite></aside>
        if trimmed.to_ascii_lowercase().starts_with("<aside") {
            self.pos += 1;
            let content = extract_tag_body(trimmed, "aside");
            let (text, credit) = split_cite(&content);
            return Some(vec![tl::enums::PageBlock::Pullquote(
                tl::types::PageBlockPullquote {
                    text: parse_rich_inline_md(&text),
                    caption: parse_rich_inline_md(&credit),
                },
            )]);
        }

        // <tg-math-block>source</tg-math-block>
        if trimmed.to_ascii_lowercase().starts_with("<tg-math-block") {
            self.pos += 1;
            let src = extract_tag_body(trimmed, "tg-math-block");
            return Some(vec![tl::enums::PageBlock::Math(tl::types::PageBlockMath {
                source: src,
            })]);
        }

        // <footer>text</footer>
        if trimmed.to_ascii_lowercase().starts_with("<footer") {
            self.pos += 1;
            let content = extract_tag_body(trimmed, "footer");
            return Some(vec![tl::enums::PageBlock::Footer(
                tl::types::PageBlockFooter {
                    text: parse_rich_inline_md(&content),
                },
            )]);
        }

        // <tg-map lat="N" long="N" zoom="N"/>
        if trimmed.to_ascii_lowercase().starts_with("<tg-map") {
            self.pos += 1;
            let (tag_name, attrs) = parse_tag(
                trimmed
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .trim_end_matches('/')
                    .trim(),
            );
            let _ = tag_name;
            let lat: f64 = attrs
                .iter()
                .find(|(k, _)| k == "lat")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0.0);
            let long: f64 = attrs
                .iter()
                .find(|(k, _)| k == "long")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0.0);
            let zoom: i32 = attrs
                .iter()
                .find(|(k, _)| k == "zoom")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(15);
            return Some(vec![tl::enums::PageBlock::Map(tl::types::PageBlockMap {
                geo: tl::enums::GeoPoint::GeoPoint(tl::types::GeoPoint {
                    lat,
                    long,
                    access_hash: 0,
                    accuracy_radius: None,
                }),
                zoom,
                w: 400,
                h: 300,
                caption: empty_caption(),
            })]);
        }

        // <figure><tg-map …/>…</figure> or <figure><img src="…"/>…</figure>
        if trimmed.to_ascii_lowercase().starts_with("<figure") {
            return Some(self.parse_figure_block());
        }

        // <a name="id"></a> - standalone anchor
        if trimmed.to_ascii_lowercase().starts_with("<a ") && trimmed.contains("name=") {
            self.pos += 1;
            let (_, attrs) = parse_tag(
                trimmed
                    .trim_start_matches('<')
                    .split('>')
                    .next()
                    .unwrap_or("")
                    .trim(),
            );
            let name = attrs
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            return Some(vec![tl::enums::PageBlock::Anchor(
                tl::types::PageBlockAnchor { name },
            )]);
        }

        // <blockquote>…</blockquote>
        if trimmed.to_ascii_lowercase().starts_with("<blockquote") {
            self.pos += 1;
            let content = extract_tag_body(trimmed, "blockquote");
            let (text, credit) = split_cite(&content);
            return Some(vec![tl::enums::PageBlock::Blockquote(
                tl::types::PageBlockBlockquote {
                    text: parse_rich_inline_md(&text),
                    caption: parse_rich_inline_md(&credit),
                },
            )]);
        }

        // <h1>…</h1> … <h6>…</h6>
        for level in 1usize..=6 {
            let tag = format!("<h{level}");
            if trimmed.to_ascii_lowercase().starts_with(&tag) {
                self.pos += 1;
                let content = extract_tag_body(trimmed, &format!("h{level}"));
                return Some(vec![heading_block(level, parse_rich_inline_md(&content))]);
            }
        }

        // <p>…</p>
        if trimmed.to_ascii_lowercase().starts_with("<p>")
            || trimmed.to_ascii_lowercase().starts_with("<p ")
        {
            self.pos += 1;
            let content = extract_tag_body(trimmed, "p");
            return Some(vec![tl::enums::PageBlock::Paragraph(
                tl::types::PageBlockParagraph {
                    text: parse_rich_inline_md(&content),
                },
            )]);
        }

        // <pre><code class="language-X">…</code></pre> or <pre>…</pre>
        if trimmed.to_ascii_lowercase().starts_with("<pre") {
            self.pos += 1;
            let (lang, code) = extract_pre_content(trimmed);
            return Some(vec![tl::enums::PageBlock::Preformatted(
                tl::types::PageBlockPreformatted {
                    text: rt_plain(code),
                    language: lang,
                },
            )]);
        }

        // <hr/>
        if trimmed == "<hr/>" || trimmed == "<hr>" || trimmed == "<hr />" {
            self.pos += 1;
            return Some(vec![tl::enums::PageBlock::Divider]);
        }

        // <ul>…</ul>
        if trimmed.to_ascii_lowercase().starts_with("<ul") {
            self.pos += 1;
            let content = self.extract_multiline_tag("ul");
            let items = parse_html_list_items(&content, false);
            return Some(vec![tl::enums::PageBlock::List(tl::types::PageBlockList {
                items,
            })]);
        }

        // <ol …>…</ol>
        if trimmed.to_ascii_lowercase().starts_with("<ol") {
            let (_, attrs) = parse_tag(
                trimmed
                    .trim_start_matches('<')
                    .split('>')
                    .next()
                    .unwrap_or("")
                    .trim(),
            );
            let start: Option<i32> = attrs
                .iter()
                .find(|(k, _)| k == "start")
                .and_then(|(_, v)| v.parse().ok());
            let reversed = attrs.iter().any(|(k, _)| k == "reversed");
            let ol_type: Option<String> = attrs
                .iter()
                .find(|(k, _)| k == "type")
                .map(|(_, v)| v.clone());
            self.pos += 1;
            let content = self.extract_multiline_tag("ol");
            let items = parse_html_ordered_list_items(&content, ol_type.as_deref());
            return Some(vec![tl::enums::PageBlock::OrderedList(
                tl::types::PageBlockOrderedList {
                    reversed,
                    items,
                    start,
                    r#type: ol_type,
                },
            )]);
        }

        // <table …>…</table>
        if trimmed.to_ascii_lowercase().starts_with("<table") {
            let (_, attrs) = parse_tag(
                trimmed
                    .trim_start_matches('<')
                    .split('>')
                    .next()
                    .unwrap_or("")
                    .trim(),
            );
            let bordered = attrs.iter().any(|(k, _)| k == "bordered");
            let striped = attrs.iter().any(|(k, _)| k == "striped");
            self.pos += 1;
            let content = self.extract_multiline_tag("table");
            let (title, rows) = parse_html_table(&content);
            return Some(vec![tl::enums::PageBlock::Table(
                tl::types::PageBlockTable {
                    bordered,
                    striped,
                    title,
                    rows,
                },
            )]);
        }

        None
    }

    fn parse_details_block(&mut self) -> Vec<tl::enums::PageBlock> {
        let line = self.lines[self.pos];
        let (_, attrs) = {
            let tag_raw = line.trim().trim_start_matches('<');
            let tag_part = tag_raw.split('>').next().unwrap_or("").trim();
            parse_tag(tag_part)
        };
        let is_open = attrs
            .iter()
            .any(|(k, v)| k == "open" || (k == "open" && v.is_empty()));
        // Check attrs for bare "open"
        let is_open = is_open
            || line.to_ascii_lowercase().contains(" open")
            || line.to_ascii_lowercase().contains(" open>");

        // Extract summary from same line or next line
        let full: String = {
            let mut lines = vec![line.to_string()];
            self.pos += 1;
            // Collect until </details>
            loop {
                match self.peek() {
                    None => break,
                    Some(l) if l.trim().eq_ignore_ascii_case("</details>") => {
                        self.pos += 1;
                        break;
                    }
                    Some(l) => {
                        lines.push(l.to_string());
                        self.pos += 1;
                    }
                }
            }
            lines.join("\n")
        };

        let summary_text = extract_between(&full, "<summary>", "</summary>").unwrap_or_default();
        let title = parse_rich_inline_md(&summary_text);

        // Parse body blocks (content after </summary>)
        let body_start = full
            .find("</summary>")
            .map(|i| i + "</summary>".len())
            .unwrap_or(full.len());
        let body_end = full.rfind("</details>").unwrap_or(full.len());
        let body = full[body_start..body_end].trim();
        let inner_blocks = parse_rich_markdown(body);

        vec![tl::enums::PageBlock::Details(tl::types::PageBlockDetails {
            open: is_open,
            blocks: inner_blocks,
            title,
        })]
    }

    fn parse_collage_block(&mut self, tag: &str) -> tl::enums::PageBlock {
        let line = self.lines[self.pos].to_string();
        // Collect until closing tag
        let close = format!("</{tag}>");
        let mut content_lines = vec![line.clone()];
        self.pos += 1;
        loop {
            match self.peek() {
                None => break,
                Some(l) if l.trim().to_ascii_lowercase().starts_with(&close) => {
                    content_lines.push(l.to_string());
                    self.pos += 1;
                    break;
                }
                Some(l) => {
                    content_lines.push(l.to_string());
                    self.pos += 1;
                }
            }
        }
        let full = content_lines.join("\n");
        let (media_items, caption) = extract_collage_items(&full);
        if tag == "tg-collage" {
            tl::enums::PageBlock::Collage(tl::types::PageBlockCollage {
                items: media_items,
                caption: caption.unwrap_or_else(empty_caption),
            })
        } else {
            tl::enums::PageBlock::Slideshow(tl::types::PageBlockSlideshow {
                items: media_items,
                caption: caption.unwrap_or_else(empty_caption),
            })
        }
    }

    fn parse_figure_block(&mut self) -> Vec<tl::enums::PageBlock> {
        let line = self.lines[self.pos].to_string();
        self.pos += 1;
        // Try to extract from single-line <figure>…</figure>
        let content = if line.contains("</figure>") {
            line.clone()
        } else {
            // Multi-line figure
            let mut lines = vec![line];
            loop {
                match self.peek() {
                    None => break,
                    Some(l) if l.trim().to_ascii_lowercase().contains("</figure>") => {
                        lines.push(l.to_string());
                        self.pos += 1;
                        break;
                    }
                    Some(l) => {
                        lines.push(l.to_string());
                        self.pos += 1;
                    }
                }
            }
            lines.join("\n")
        };

        // <figcaption>caption<cite>credit</cite></figcaption>
        let caption_raw =
            extract_between(&content, "<figcaption>", "</figcaption>").unwrap_or_default();
        let (cap_text, cap_credit) = split_cite(&caption_raw);
        let cap = if cap_text.is_empty() {
            empty_caption()
        } else {
            caption_text_credit(
                parse_rich_inline_md(&cap_text),
                parse_rich_inline_md(&cap_credit),
            )
        };

        let spoiler = content.contains("tg-spoiler");

        // <tg-map …/>
        if content.contains("<tg-map") {
            let map_part = extract_between(&content, "<tg-map", "/>").unwrap_or_default();
            let (_, attrs) = parse_tag(&format!("tg-map {map_part}"));
            let lat: f64 = attrs
                .iter()
                .find(|(k, _)| k == "lat")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0.0);
            let long: f64 = attrs
                .iter()
                .find(|(k, _)| k == "long")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0.0);
            let zoom: i32 = attrs
                .iter()
                .find(|(k, _)| k == "zoom")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(15);
            return vec![tl::enums::PageBlock::Map(tl::types::PageBlockMap {
                geo: tl::enums::GeoPoint::GeoPoint(tl::types::GeoPoint {
                    lat,
                    long,
                    access_hash: 0,
                    accuracy_radius: None,
                }),
                zoom,
                w: 400,
                h: 300,
                caption: cap,
            })];
        }

        // <img src="…"/> or <video src="…"/> or <audio src="…"/>
        let src = extract_src_from_figure(&content);
        if let Some(url) = src {
            return vec![media_block(&url, cap, spoiler)];
        }

        vec![]
    }

    fn extract_multiline_tag(&mut self, tag: &str) -> String {
        let close = format!("</{tag}>");
        let mut lines = Vec::new();
        loop {
            match self.peek() {
                None => break,
                Some(l) => {
                    let owned = l.to_string();
                    self.pos += 1;
                    if owned.trim().to_ascii_lowercase().contains(&close) {
                        lines.push(owned);
                        break;
                    }
                    lines.push(owned);
                }
            }
        }
        lines.join("\n")
    }
}

// Heading level → PageBlock
fn heading_block(level: usize, rt: tl::enums::RichText) -> tl::enums::PageBlock {
    match level {
        1 => tl::enums::PageBlock::Heading1(tl::types::PageBlockHeading1 { text: rt }),
        2 => tl::enums::PageBlock::Heading2(tl::types::PageBlockHeading2 { text: rt }),
        3 => tl::enums::PageBlock::Heading3(tl::types::PageBlockHeading3 { text: rt }),
        4 => tl::enums::PageBlock::Heading4(tl::types::PageBlockHeading4 { text: rt }),
        5 => tl::enums::PageBlock::Heading5(tl::types::PageBlockHeading5 { text: rt }),
        _ => tl::enums::PageBlock::Heading6(tl::types::PageBlockHeading6 { text: rt }),
    }
}

fn list_unordered_start(line: &str) -> bool {
    let t = line.trim_start();
    (t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ "))
        && !matches!(t, "---" | "***" | "___")
}

fn list_ordered_start(line: &str) -> bool {
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

/// Returns `(Some(checked), text)` for task items, `(None, text)` for plain.
fn parse_list_item_unordered(line: &str) -> Option<(Option<bool>, &str)> {
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

fn parse_list_item_ordered(line: &str) -> Option<(i32, &str)> {
    let t = line.trim_start();
    let dot = t.find(['.', ')'])?;
    let num: i32 = t[..dot].parse().ok()?;
    let rest = t[dot + 1..].trim_start();
    Some((num, rest))
}

fn split_table_row(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').collect()
}

fn is_block_html_tag(s: &str) -> bool {
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

fn try_parse_media_line(line: &str) -> Option<tl::enums::PageBlock> {
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

fn extract_title_from_url_part(s: &str) -> String {
    // url "title" → title
    if let Some(q) = s.find(" \"") {
        let after = &s[q + 2..];
        if let Some(close) = after.rfind('"') {
            return after[..close].to_string();
        }
    }
    String::new()
}

fn extract_tag_body(html: &str, tag: &str) -> String {
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

fn extract_between(s: &str, open: &str, close: &str) -> Option<String> {
    let lo = s.to_ascii_lowercase();
    let start = lo.find(&open.to_ascii_lowercase())? + open.len();
    let end = lo[start..]
        .find(&close.to_ascii_lowercase())
        .map(|i| start + i)?;
    Some(s[start..end].to_string())
}

fn split_cite(html: &str) -> (String, String) {
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

fn extract_pre_content(html: &str) -> (String, String) {
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

fn extract_collage_items(
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

fn extract_src_from_figure(html: &str) -> Option<String> {
    for part in html.split('<') {
        if (part.starts_with("img ") || part.starts_with("video ") || part.starts_with("audio "))
            && let Some(src) = extract_attr_value(part, "src")
        {
            return Some(src);
        }
    }
    None
}

fn extract_attr_value(tag_body: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag_body.find(&needle)? + needle.len();
    let end = tag_body[start..].find('"')? + start;
    Some(tag_body[start..end].to_string())
}

fn parse_html_list_items(html: &str, _ordered: bool) -> Vec<tl::enums::PageListItem> {
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

fn parse_html_ordered_list_items(
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

fn parse_html_table(html: &str) -> (tl::enums::RichText, Vec<tl::enums::PageTableRow>) {
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

fn parse_html_table_cells(html: &str) -> Vec<tl::enums::PageTableCell> {
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

// Rich HTML parser: `&str` → `Vec<PageBlock>`

/// Parse **Rich HTML** into a list of `PageBlock`s for use in `InputRichMessage`.
///
/// All block and inline tags from the Bot API Rich HTML spec are supported.
pub fn parse_rich_html(html: &str) -> Vec<tl::enums::PageBlock> {
    RichHtmlParser::new(html).parse()
}

struct RichHtmlParser {
    html: String,
    pos: usize,
}

impl RichHtmlParser {
    fn new(html: &str) -> Self {
        Self {
            html: html.to_string(),
            pos: 0,
        }
    }

    fn remaining(&self) -> &str {
        &self.html[self.pos..]
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.html.len() {
            let c = self.html.as_bytes()[self.pos];
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse(mut self) -> Vec<tl::enums::PageBlock> {
        let mut blocks = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.html.len() {
                break;
            }
            if self.remaining().starts_with('<')
                && let Some(block) = self.try_parse_block_tag()
            {
                blocks.extend(block);
                continue;
            }
            // Text content at top level → paragraph
            if let Some(para) = self.parse_text_paragraph() {
                blocks.push(para);
            }
        }
        blocks
    }

    fn try_parse_block_tag(&mut self) -> Option<Vec<tl::enums::PageBlock>> {
        let rem = self.remaining();
        let lower = rem.to_ascii_lowercase();

        macro_rules! heading {
            ($tag:literal, $level:expr) => {
                if lower.starts_with(concat!("<", $tag, ">"))
                    || lower.starts_with(concat!("<", $tag, " "))
                {
                    let body = self.consume_tag($tag)?;
                    return Some(vec![heading_block($level, parse_rich_html_inline(&body))]);
                }
            };
        }

        heading!("h1", 1);
        heading!("h2", 2);
        heading!("h3", 3);
        heading!("h4", 4);
        heading!("h5", 5);
        heading!("h6", 6);

        if lower.starts_with("<p>") || lower.starts_with("<p ") {
            let body = self.consume_tag("p")?;
            return Some(vec![tl::enums::PageBlock::Paragraph(
                tl::types::PageBlockParagraph {
                    text: parse_rich_html_inline(&body),
                },
            )]);
        }

        if lower.starts_with("<pre>") || lower.starts_with("<pre>") || lower.starts_with("<pre ") {
            let body = self.consume_tag("pre")?;
            let (lang, code) = extract_pre_content_from_body(&body);
            return Some(vec![tl::enums::PageBlock::Preformatted(
                tl::types::PageBlockPreformatted {
                    text: rt_plain(code),
                    language: lang,
                },
            )]);
        }

        if lower.starts_with("<footer>") || lower.starts_with("<footer ") {
            let body = self.consume_tag("footer")?;
            return Some(vec![tl::enums::PageBlock::Footer(
                tl::types::PageBlockFooter {
                    text: parse_rich_html_inline(&body),
                },
            )]);
        }

        if lower.starts_with("<hr") {
            self.consume_until('>');
            self.pos += 1;
            return Some(vec![tl::enums::PageBlock::Divider]);
        }

        if lower.starts_with("<blockquote") {
            let body = self.consume_tag("blockquote")?;
            let (text, credit) = split_cite(&body);
            return Some(vec![tl::enums::PageBlock::Blockquote(
                tl::types::PageBlockBlockquote {
                    text: parse_rich_html_inline(&text),
                    caption: parse_rich_html_inline(&credit),
                },
            )]);
        }

        if lower.starts_with("<aside") {
            let body = self.consume_tag("aside")?;
            let (text, credit) = split_cite(&body);
            return Some(vec![tl::enums::PageBlock::Pullquote(
                tl::types::PageBlockPullquote {
                    text: parse_rich_html_inline(&text),
                    caption: parse_rich_html_inline(&credit),
                },
            )]);
        }

        if lower.starts_with("<ul") {
            let body = self.consume_tag("ul")?;
            let items = parse_html_list_items(&body, false);
            return Some(vec![tl::enums::PageBlock::List(tl::types::PageBlockList {
                items,
            })]);
        }

        if lower.starts_with("<ol") {
            let tag_open = rem.split('>').next().unwrap_or("").to_string();
            let (_, attrs) = parse_tag(tag_open.trim_start_matches('<'));
            let start: Option<i32> = attrs
                .iter()
                .find(|(k, _)| k == "start")
                .and_then(|(_, v)| v.parse().ok());
            let reversed = attrs.iter().any(|(k, _)| k == "reversed");
            let ol_type: Option<String> = attrs
                .iter()
                .find(|(k, _)| k == "type")
                .map(|(_, v)| v.clone());
            let body = self.consume_tag("ol")?;
            let items = parse_html_ordered_list_items(&body, ol_type.as_deref());
            return Some(vec![tl::enums::PageBlock::OrderedList(
                tl::types::PageBlockOrderedList {
                    reversed,
                    items,
                    start,
                    r#type: ol_type,
                },
            )]);
        }

        if lower.starts_with("<table") {
            let tag_open = rem.split('>').next().unwrap_or("").to_string();
            let (_, attrs) = parse_tag(tag_open.trim_start_matches('<'));
            let bordered = attrs.iter().any(|(k, _)| k == "bordered");
            let striped = attrs.iter().any(|(k, _)| k == "striped");
            let body = self.consume_tag("table")?;
            let (title, rows) = parse_html_table(&body);
            return Some(vec![tl::enums::PageBlock::Table(
                tl::types::PageBlockTable {
                    bordered,
                    striped,
                    title,
                    rows,
                },
            )]);
        }

        if lower.starts_with("<details") {
            let is_open_hint = rem.to_ascii_lowercase().starts_with("<details open");
            let full = self.consume_tag("details")?;
            let is_open = is_open_hint || full.starts_with("open");
            let summary = extract_between(&full, "<summary>", "</summary>").unwrap_or_default();
            let body_start = full
                .find("</summary>")
                .map(|i| i + "</summary>".len())
                .unwrap_or(full.len());
            let inner = parse_rich_html(full[body_start..].trim());
            return Some(vec![tl::enums::PageBlock::Details(
                tl::types::PageBlockDetails {
                    open: is_open,
                    blocks: inner,
                    title: parse_rich_html_inline(&summary),
                },
            )]);
        }

        if lower.starts_with("<img ") {
            let tag_raw = self.consume_self_closing_tag();
            let (_, attrs) = parse_tag(&tag_raw);
            let src = attrs
                .iter()
                .find(|(k, _)| k == "src")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let spoiler = attrs.iter().any(|(k, _)| k == "tg-spoiler");
            if !src.is_empty() {
                return Some(vec![media_block(&src, empty_caption(), spoiler)]);
            }
            return Some(vec![]);
        }

        if lower.starts_with("<video ") {
            let tag_raw = self.consume_self_closing_or_pair("video");
            let (_, attrs) = parse_tag(&tag_raw);
            let src = attrs
                .iter()
                .find(|(k, _)| k == "src")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let spoiler = attrs.iter().any(|(k, _)| k == "tg-spoiler");
            if !src.is_empty() {
                return Some(vec![media_block(&src, empty_caption(), spoiler)]);
            }
            return Some(vec![]);
        }

        if lower.starts_with("<audio ") {
            let tag_raw = self.consume_self_closing_or_pair("audio");
            let (_, attrs) = parse_tag(&tag_raw);
            let src = attrs
                .iter()
                .find(|(k, _)| k == "src")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            if !src.is_empty() {
                return Some(vec![media_block(&src, empty_caption(), false)]);
            }
            return Some(vec![]);
        }

        if lower.starts_with("<figure") {
            let body = self.consume_tag("figure")?;
            let caption_raw =
                extract_between(&body, "<figcaption>", "</figcaption>").unwrap_or_default();
            let (cap_t, cap_cr) = split_cite(&caption_raw);
            let cap = if cap_t.is_empty() {
                empty_caption()
            } else {
                caption_text_credit(
                    parse_rich_html_inline(&cap_t),
                    parse_rich_html_inline(&cap_cr),
                )
            };
            let spoiler = body.contains("tg-spoiler");

            if body.to_ascii_lowercase().contains("<tg-map") {
                let map_inner = extract_between(&body, "<tg-map", "/>").unwrap_or_default();
                let (_, attrs) = parse_tag(&format!("tg-map {map_inner}"));
                let lat: f64 = attrs
                    .iter()
                    .find(|(k, _)| k == "lat")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0.0);
                let long: f64 = attrs
                    .iter()
                    .find(|(k, _)| k == "long")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0.0);
                let zoom: i32 = attrs
                    .iter()
                    .find(|(k, _)| k == "zoom")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(15);
                return Some(vec![tl::enums::PageBlock::Map(tl::types::PageBlockMap {
                    geo: tl::enums::GeoPoint::GeoPoint(tl::types::GeoPoint {
                        lat,
                        long,
                        access_hash: 0,
                        accuracy_radius: None,
                    }),
                    zoom,
                    w: 400,
                    h: 300,
                    caption: cap,
                })]);
            }

            let src = extract_src_from_figure(&body);
            if let Some(url) = src {
                return Some(vec![media_block(&url, cap, spoiler)]);
            }
            return Some(vec![]);
        }

        if lower.starts_with("<tg-collage") {
            let body = self.consume_tag("tg-collage")?;
            let (items, cap) = extract_collage_items(&body);
            return Some(vec![tl::enums::PageBlock::Collage(
                tl::types::PageBlockCollage {
                    items,
                    caption: cap.unwrap_or_else(empty_caption),
                },
            )]);
        }

        if lower.starts_with("<tg-slideshow") {
            let body = self.consume_tag("tg-slideshow")?;
            let (items, cap) = extract_collage_items(&body);
            return Some(vec![tl::enums::PageBlock::Slideshow(
                tl::types::PageBlockSlideshow {
                    items,
                    caption: cap.unwrap_or_else(empty_caption),
                },
            )]);
        }

        if lower.starts_with("<tg-map") {
            let tag_raw = self.consume_self_closing_tag();
            let (_, attrs) = parse_tag(&tag_raw);
            let lat: f64 = attrs
                .iter()
                .find(|(k, _)| k == "lat")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0.0);
            let long: f64 = attrs
                .iter()
                .find(|(k, _)| k == "long")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0.0);
            let zoom: i32 = attrs
                .iter()
                .find(|(k, _)| k == "zoom")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(15);
            return Some(vec![tl::enums::PageBlock::Map(tl::types::PageBlockMap {
                geo: tl::enums::GeoPoint::GeoPoint(tl::types::GeoPoint {
                    lat,
                    long,
                    access_hash: 0,
                    accuracy_radius: None,
                }),
                zoom,
                w: 400,
                h: 300,
                caption: empty_caption(),
            })]);
        }

        if lower.starts_with("<tg-math-block") {
            let body = self.consume_tag("tg-math-block")?;
            return Some(vec![tl::enums::PageBlock::Math(tl::types::PageBlockMath {
                source: body,
            })]);
        }

        if lower.starts_with("<a ") && lower.contains("name=") {
            // Standalone anchor: <a name="id"></a>
            let tag_raw = self.consume_self_closing_or_pair("a");
            let (_, attrs) = parse_tag(&tag_raw);
            let name = attrs
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            if !name.is_empty() {
                return Some(vec![tl::enums::PageBlock::Anchor(
                    tl::types::PageBlockAnchor { name },
                )]);
            }
            return Some(vec![]);
        }

        // Skip unknown/comment/doctype tags
        if lower.starts_with("<!--") || lower.starts_with("<!") {
            self.consume_until('>');
            self.pos = (self.pos + 1).min(self.html.len());
            return Some(vec![]);
        }

        None
    }

    fn consume_tag(&mut self, tag: &str) -> Option<String> {
        // Move past the opening tag
        let open_end = self.remaining().find('>')?;
        self.pos += open_end + 1;
        let close_tag = format!("</{tag}>");
        let close_pos = self.remaining().to_ascii_lowercase().find(&close_tag)?;
        let body = self.remaining()[..close_pos].to_string();
        self.pos += close_pos + close_tag.len();
        Some(body)
    }

    fn consume_self_closing_tag(&mut self) -> String {
        let end = self.remaining().find('>').unwrap_or(self.remaining().len());
        let tag_raw = self.remaining()[1..end]
            .trim_end_matches('/')
            .trim()
            .to_string();
        self.pos += end + 1;
        tag_raw
    }

    fn consume_self_closing_or_pair(&mut self, tag: &str) -> String {
        let rem = self.remaining();
        // Check if it's self-closing or has a close tag in the same stretch
        let open_end = rem.find('>').unwrap_or(rem.len());
        let is_self = rem[..open_end].ends_with('/');
        let tag_raw = rem[1..open_end].trim_end_matches('/').trim().to_string();
        self.pos += open_end + 1;
        if !is_self {
            let close_tag = format!("</{tag}>");
            if let Some(end) = self.remaining().to_ascii_lowercase().find(&close_tag) {
                self.pos += end + close_tag.len();
            }
        }
        tag_raw
    }

    fn consume_until(&mut self, ch: char) {
        while self.pos < self.html.len() {
            if self.html.as_bytes()[self.pos] == ch as u8 {
                break;
            }
            self.pos += 1;
        }
    }

    fn parse_text_paragraph(&mut self) -> Option<tl::enums::PageBlock> {
        let start = self.pos;
        while self.pos < self.html.len() {
            let rem = self.remaining();
            if rem.starts_with('<') {
                // Peek at the tag: if it's a block tag, stop
                let lower = rem.to_ascii_lowercase();
                let is_block = is_block_html_tag(&lower);
                if is_block {
                    break;
                }
                // Inline tag: include it as-is and continue
                let end = rem.find('>').unwrap_or(rem.len());
                self.pos += end + 1;
            } else {
                self.pos += 1;
            }
        }
        if self.pos == start {
            return None;
        }
        let text_raw = &self.html[start..self.pos];
        let decoded = decode_html_entities(text_raw);
        if decoded.trim().is_empty() {
            return None;
        }
        Some(tl::enums::PageBlock::Paragraph(
            tl::types::PageBlockParagraph {
                text: parse_rich_html_inline(&decoded),
            },
        ))
    }
}

/// Parse an HTML inline string into a `RichText` tree.
/// Handles all inline tags: b, strong, i, em, u, ins, s, del, strike,
/// code, mark, tg-spoiler, sub, sup, a, tg-emoji, tg-time, tg-math, tg-reference.
pub fn parse_rich_html_inline(html: &str) -> tl::enums::RichText {
    let chars: Vec<char> = html.chars().collect();
    let mut parts = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let n = chars.len();

    macro_rules! flush {
        () => {
            if !buf.is_empty() {
                parts.push(rt_plain(decode_html_entities(&std::mem::take(&mut buf))));
            }
        };
    }

    while i < n {
        if chars[i] == '&' {
            // HTML entity: collect until `;`
            let mut j = i + 1;
            while j < n && chars[j] != ';' && chars[j] != ' ' {
                j += 1;
            }
            if j < n && chars[j] == ';' {
                let entity: String = chars[i..=j].iter().collect();
                buf.push_str(&decode_html_entities(&entity));
                i = j + 1;
                continue;
            }
        }

        if chars[i] != '<' {
            buf.push(chars[i]);
            i += 1;
            continue;
        }

        // Try to parse as inline HTML tag
        let remaining: String = chars[i..].iter().collect();
        if let Some((consumed, rt)) = try_parse_html_inline_tag(&chars, i, n) {
            flush!();
            parts.push(rt);
            i = consumed;
            continue;
        }

        // Not recognised - emit as text
        buf.push(chars[i]);
        i += 1;
        let _ = remaining;
    }
    flush!();
    rt_concat(parts)
}

fn extract_pre_content_from_body(body: &str) -> (String, String) {
    // <code class="language-X">…</code>
    let lo = body.to_ascii_lowercase();
    if lo.contains("<code") {
        let lang = extract_between(body, "class=\"language-", "\"").unwrap_or_default();
        let code_start = lo.find('>').map(|i| i + 1).unwrap_or(0);
        let code = extract_between(body, ">", "</code>")
            .or_else(|| {
                extract_between(body, "<code", "</code>").map(|c| {
                    let ci = c.find('>').map(|i| i + 1).unwrap_or(0);
                    c[ci..].to_string()
                })
            })
            .unwrap_or_else(|| body[code_start..].to_string());
        return (lang, decode_html_entities(&code));
    }
    (String::new(), decode_html_entities(body))
}

// Tests for rich parsers

#[cfg(test)]
mod rich_tests {
    use super::*;

    #[test]
    fn rich_md_heading1() {
        let blocks = parse_rich_markdown("# Hello");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Heading1(_)));
        if let tl::enums::PageBlock::Heading1(h) = &blocks[0] {
            assert!(matches!(h.text, tl::enums::RichText::TextPlain(_)));
        }
    }

    #[test]
    fn rich_md_heading6() {
        let blocks = parse_rich_markdown("###### Deep");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Heading6(_)));
    }

    #[test]
    fn rich_md_paragraph() {
        let blocks = parse_rich_markdown("Hello world");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Paragraph(_)));
    }

    #[test]
    fn rich_md_code_block() {
        let blocks = parse_rich_markdown("```python\nprint('hi')\n```");
        if let tl::enums::PageBlock::Preformatted(p) = &blocks[0] {
            assert_eq!(p.language, "python");
            assert!(matches!(p.text, tl::enums::RichText::TextPlain(_)));
        } else {
            panic!("expected Preformatted");
        }
    }

    #[test]
    fn rich_md_math_block_backtick() {
        let blocks = parse_rich_markdown("```math\nE = mc^2\n```");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Math(_)));
        if let tl::enums::PageBlock::Math(m) = &blocks[0] {
            assert_eq!(m.source, "E = mc^2");
        }
    }

    #[test]
    fn rich_md_divider() {
        let blocks = parse_rich_markdown("---");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Divider));
    }

    #[test]
    fn rich_md_unordered_list() {
        let blocks = parse_rich_markdown("- item 1\n- item 2");
        assert!(matches!(blocks[0], tl::enums::PageBlock::List(_)));
        if let tl::enums::PageBlock::List(l) = &blocks[0] {
            assert_eq!(l.items.len(), 2);
        }
    }

    #[test]
    fn rich_md_task_list() {
        let blocks = parse_rich_markdown("- [ ] todo\n- [x] done");
        if let tl::enums::PageBlock::List(l) = &blocks[0] {
            assert!(
                matches!(l.items[0], tl::enums::PageListItem::Text(ref t) if t.checkbox && !t.checked)
            );
            assert!(
                matches!(l.items[1], tl::enums::PageListItem::Text(ref t) if t.checkbox && t.checked)
            );
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn rich_md_ordered_list() {
        let blocks = parse_rich_markdown("1. first\n2. second");
        assert!(matches!(blocks[0], tl::enums::PageBlock::OrderedList(_)));
        if let tl::enums::PageBlock::OrderedList(l) = &blocks[0] {
            assert_eq!(l.items.len(), 2);
        }
    }

    #[test]
    fn rich_md_blockquote() {
        let blocks = parse_rich_markdown(">Hello\n>World");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Blockquote(_)));
    }

    #[test]
    fn rich_md_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let blocks = parse_rich_markdown(md);
        assert!(matches!(blocks[0], tl::enums::PageBlock::Table(_)));
        if let tl::enums::PageBlock::Table(t) = &blocks[0] {
            assert_eq!(t.rows.len(), 2); // header + data
        }
    }

    #[test]
    fn rich_md_inline_bold() {
        let rt = parse_rich_inline_md("**bold**");
        assert!(matches!(rt, tl::enums::RichText::TextBold(_)));
    }

    #[test]
    fn rich_md_inline_italic() {
        let rt = parse_rich_inline_md("*italic*");
        assert!(matches!(rt, tl::enums::RichText::TextItalic(_)));
    }

    #[test]
    fn rich_md_inline_code() {
        let rt = parse_rich_inline_md("`code`");
        assert!(matches!(rt, tl::enums::RichText::TextFixed(_)));
    }

    #[test]
    fn rich_md_inline_mark() {
        let rt = parse_rich_inline_md("==marked==");
        assert!(matches!(rt, tl::enums::RichText::TextMarked(_)));
    }

    #[test]
    fn rich_md_inline_spoiler() {
        let rt = parse_rich_inline_md("||secret||");
        assert!(matches!(rt, tl::enums::RichText::TextSpoiler(_)));
    }

    #[test]
    fn rich_md_inline_strike() {
        let rt = parse_rich_inline_md("~~strike~~");
        assert!(matches!(rt, tl::enums::RichText::TextStrike(_)));
    }

    #[test]
    fn rich_md_inline_url() {
        let rt = parse_rich_inline_md("[click](https://t.me/)");
        assert!(matches!(rt, tl::enums::RichText::TextUrl(_)));
    }

    #[test]
    fn rich_md_inline_mention() {
        let rt = parse_rich_inline_md("[User](tg://user?id=42)");
        assert!(matches!(rt, tl::enums::RichText::TextMentionName(_)));
        if let tl::enums::RichText::TextMentionName(m) = rt {
            assert_eq!(m.user_id, 42);
        }
    }

    #[test]
    fn rich_md_inline_email_link() {
        let rt = parse_rich_inline_md("[mail](mailto:user@example.com)");
        assert!(matches!(rt, tl::enums::RichText::TextEmail(_)));
    }

    #[test]
    fn rich_md_inline_phone_link() {
        let rt = parse_rich_inline_md("[call](tel:+123456789)");
        assert!(matches!(rt, tl::enums::RichText::TextPhone(_)));
    }

    #[test]
    fn rich_md_inline_custom_emoji() {
        let rt = parse_rich_inline_md("![👍](tg://emoji?id=5368324170671202286)");
        assert!(matches!(rt, tl::enums::RichText::TextCustomEmoji(_)));
        if let tl::enums::RichText::TextCustomEmoji(e) = rt {
            assert_eq!(e.document_id, 5368324170671202286);
        }
    }

    #[test]
    fn rich_md_inline_math() {
        let rt = parse_rich_inline_md("$x^2 + y^2$");
        assert!(matches!(rt, tl::enums::RichText::TextMath(_)));
        if let tl::enums::RichText::TextMath(m) = rt {
            assert_eq!(m.source, "x^2 + y^2");
        }
    }

    #[test]
    fn rich_md_inline_html_underline() {
        let rt = parse_rich_inline_md("<u>underlined</u>");
        assert!(matches!(rt, tl::enums::RichText::TextUnderline(_)));
    }

    #[test]
    fn rich_md_inline_html_sub() {
        let rt = parse_rich_inline_md("<sub>sub</sub>");
        assert!(matches!(rt, tl::enums::RichText::TextSubscript(_)));
    }

    #[test]
    fn rich_md_inline_html_sup() {
        let rt = parse_rich_inline_md("<sup>sup</sup>");
        assert!(matches!(rt, tl::enums::RichText::TextSuperscript(_)));
    }

    #[test]
    fn rich_md_inline_tg_spoiler_html() {
        let rt = parse_rich_inline_md("<tg-spoiler>hidden</tg-spoiler>");
        assert!(matches!(rt, tl::enums::RichText::TextSpoiler(_)));
    }

    #[test]
    fn rich_html_heading() {
        let blocks = parse_rich_html("<h2>World</h2>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Heading2(_)));
    }

    #[test]
    fn rich_html_paragraph() {
        let blocks = parse_rich_html("<p>Hello</p>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Paragraph(_)));
    }

    #[test]
    fn rich_html_preformatted() {
        let blocks = parse_rich_html("<pre><code class=\"language-rust\">fn main(){}</code></pre>");
        if let tl::enums::PageBlock::Preformatted(p) = &blocks[0] {
            assert_eq!(p.language, "rust");
        } else {
            panic!(
                "expected Preformatted, got {:?}",
                blocks.get(0).map(|_| "block")
            );
        }
    }

    #[test]
    fn rich_html_blockquote() {
        let blocks = parse_rich_html("<blockquote>Quote<cite>Author</cite></blockquote>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Blockquote(_)));
        if let tl::enums::PageBlock::Blockquote(b) = &blocks[0] {
            assert!(!matches!(b.caption, tl::enums::RichText::TextEmpty));
        }
    }

    #[test]
    fn rich_html_aside_pullquote() {
        let blocks = parse_rich_html("<aside>Pull quote<cite>The Author</cite></aside>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Pullquote(_)));
    }

    #[test]
    fn rich_html_hr_divider() {
        let blocks = parse_rich_html("<hr/>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Divider));
    }

    #[test]
    fn rich_html_unordered_list() {
        let blocks = parse_rich_html("<ul><li>a</li><li>b</li></ul>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::List(_)));
        if let tl::enums::PageBlock::List(l) = &blocks[0] {
            assert_eq!(l.items.len(), 2);
        }
    }

    #[test]
    fn rich_html_ordered_list() {
        let blocks = parse_rich_html("<ol><li>first</li><li>second</li></ol>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::OrderedList(_)));
    }

    #[test]
    fn rich_html_table() {
        let blocks = parse_rich_html(
            "<table><tr><th>H1</th><th>H2</th></tr><tr><td>v1</td><td>v2</td></tr></table>",
        );
        assert!(matches!(blocks[0], tl::enums::PageBlock::Table(_)));
        if let tl::enums::PageBlock::Table(t) = &blocks[0] {
            assert_eq!(t.rows.len(), 2);
        }
    }

    #[test]
    fn rich_html_details() {
        let blocks = parse_rich_html("<details open><summary>Title</summary>Content</details>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Details(_)));
        if let tl::enums::PageBlock::Details(d) = &blocks[0] {
            assert!(d.open);
        }
    }

    #[test]
    fn rich_html_map() {
        let blocks = parse_rich_html("<tg-map lat=\"41.9\" long=\"12.5\" zoom=\"14\"/>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Map(_)));
        if let tl::enums::PageBlock::Map(m) = &blocks[0] {
            assert_eq!(m.zoom, 14);
        }
    }

    #[test]
    fn rich_html_math_block() {
        let blocks = parse_rich_html("<tg-math-block>E = mc^2</tg-math-block>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Math(_)));
        if let tl::enums::PageBlock::Math(m) = &blocks[0] {
            assert_eq!(m.source, "E = mc^2");
        }
    }

    #[test]
    fn rich_html_inline_bold() {
        let rt = parse_rich_html_inline("<b>bold</b>");
        assert!(matches!(rt, tl::enums::RichText::TextBold(_)));
    }

    #[test]
    fn rich_html_inline_spoiler() {
        let rt = parse_rich_html_inline("<tg-spoiler>secret</tg-spoiler>");
        assert!(matches!(rt, tl::enums::RichText::TextSpoiler(_)));
    }

    #[test]
    fn rich_html_inline_custom_emoji() {
        let rt = parse_rich_html_inline("<tg-emoji emoji-id=\"999\">👍</tg-emoji>");
        assert!(matches!(rt, tl::enums::RichText::TextCustomEmoji(_)));
        if let tl::enums::RichText::TextCustomEmoji(e) = rt {
            assert_eq!(e.document_id, 999);
        }
    }

    #[test]
    fn rich_html_inline_tg_time() {
        let rt = parse_rich_html_inline(
            "<tg-time unix=\"1647531900\" format=\"wDT\">22:45 tomorrow</tg-time>",
        );
        assert!(matches!(rt, tl::enums::RichText::TextDate(_)));
        if let tl::enums::RichText::TextDate(d) = rt {
            assert_eq!(d.date, 1647531900);
            assert!(d.day_of_week);
        }
    }

    #[test]
    fn rich_html_photo_block() {
        let blocks = parse_rich_html("<img src=\"https://telegram.org/example/photo.jpg\"/>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Photo(_)));
    }

    #[test]
    fn rich_html_video_block() {
        let blocks =
            parse_rich_html("<video src=\"https://telegram.org/example/video.mp4\"></video>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Video(_)));
    }

    #[test]
    fn rich_html_audio_block() {
        let blocks =
            parse_rich_html("<audio src=\"https://telegram.org/example/audio.mp3\"></audio>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Audio(_)));
    }

    #[test]
    fn rich_html_collage() {
        let blocks = parse_rich_html(
            "<tg-collage><img src=\"https://telegram.org/example/photo.jpg\"/><video src=\"https://telegram.org/example/video.mp4\"/></tg-collage>",
        );
        assert!(matches!(blocks[0], tl::enums::PageBlock::Collage(_)));
        if let tl::enums::PageBlock::Collage(c) = &blocks[0] {
            assert_eq!(c.items.len(), 2);
        }
    }

    #[test]
    fn rich_html_slideshow() {
        let blocks = parse_rich_html(
            "<tg-slideshow><img src=\"https://telegram.org/example/photo.jpg\"/><video src=\"https://telegram.org/example/video.mp4\"/></tg-slideshow>",
        );
        assert!(matches!(blocks[0], tl::enums::PageBlock::Slideshow(_)));
    }

    #[test]
    fn rich_html_footer() {
        let blocks = parse_rich_html("<footer>Footer text</footer>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Footer(_)));
    }

    #[test]
    fn rich_html_anchor_block() {
        let blocks = parse_rich_html("<a name=\"chapter-1\"></a>");
        assert!(matches!(blocks[0], tl::enums::PageBlock::Anchor(_)));
        if let tl::enums::PageBlock::Anchor(a) = &blocks[0] {
            assert_eq!(a.name, "chapter-1");
        }
    }
}
