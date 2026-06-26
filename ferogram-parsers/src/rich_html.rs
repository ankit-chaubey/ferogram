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

use crate::rich_common::*;

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
