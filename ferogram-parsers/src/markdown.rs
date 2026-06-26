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
