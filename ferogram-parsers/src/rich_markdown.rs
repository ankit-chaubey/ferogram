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
