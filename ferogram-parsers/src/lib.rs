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

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-parsers/0.6.2")]
//! Telegram HTML and Markdown entity parsers for ferogram.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Converts Telegram-flavoured Markdown and HTML into the `(plain_text,
//! entities)` pair that the Telegram API expects, and generates those formats
//! back from entities. Also provides rich-text parsers that produce
//! `Vec<PageBlock>` for Telegraph-style structured content.
//!
//! Most users reach this through the `ferogram` crate's `InputMessage`
//! builder (`.markdown()` / `.html()`). Use `ferogram-parsers` directly only
//! when you need the parse/generate functions without the full client.
//!
//! # What's in here
//!
//! - **[`parse_markdown`]** / **[`generate_markdown`]**: Parse and generate
//!   Telegram MarkdownV2. Supports bold, italic, underline, strikethrough,
//!   spoiler, inline code, code blocks with language, blockquotes (regular
//!   and expandable), text URLs, user mentions, and custom emoji. All offsets
//!   are in UTF-16 code units as required by the Telegram API.
//! - **[`parse_markdown_v1`]** (deprecated): Legacy MarkdownV1 retained for
//!   backward compatibility. Prefer V2 for new code.
//! - **[`parse_html`]** / **[`generate_html`]**: Parse and generate
//!   Telegram Bot API HTML. Recognises `<b>`, `<i>`, `<u>`, `<s>`,
//!   `<code>`, `<pre>`, `<a>`, `<blockquote>`, `<tg-spoiler>`,
//!   `<tg-emoji>`, `<tg-time>`, and their aliases. The `html5ever` feature
//!   switches to a spec-compliant parser for malformed input.
//! - **[`parse_rich_markdown`]** / **[`parse_rich_inline_md`]**: Parse
//!   extended Markdown into `Vec<PageBlock>` / `RichText` for Telegraph
//!   articles. Supports headings (H1–H6), paragraphs, ordered and unordered
//!   lists, task lists, tables, blockquotes, pull quotes, dividers, fenced
//!   code blocks, and math blocks.
//! - **[`parse_rich_html`]** / **[`parse_rich_html_inline`]**: Same as the
//!   Markdown rich parsers but driven by HTML input (`<h1>`–`<h6>`, `<ul>`,
//!   `<ol>`, `<table>`, `<details>`, `<tg-map>`, `<tg-math-block>`, etc.).
//!
//! # Example: Markdown round-trip
//!
//! ```rust
//! use ferogram_parsers::{parse_markdown, generate_markdown};
//!
//! let (text, entities) = parse_markdown("Hello **world**!");
//! assert_eq!(text, "Hello world!");
//!
//! let back = generate_markdown(&text, &entities);
//! assert_eq!(back, "Hello *world*!");
//! ```
//!
//! # Example: HTML to entities
//!
//! ```rust
//! use ferogram_parsers::parse_html;
//!
//! let (text, entities) = parse_html("<b>bold</b> and <i>italic</i>");
//! assert_eq!(text, "bold and italic");
//! assert_eq!(entities.len(), 2);
//! ```
//!
//! # Features
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `html` | yes | Built-in hand-rolled HTML parser |
//! | `html5ever` | no | Replaces the built-in parser with `html5ever` for spec-compliant handling of malformed HTML |

#![deny(unsafe_code)]

mod html;
mod markdown;
mod rich_common;
mod rich_html;
mod rich_markdown;

#[allow(deprecated)]
pub use markdown::{
    generate_markdown, generate_markdown_v2, parse_markdown, parse_markdown_v1, parse_markdown_v2,
};

pub use html::{generate_html, parse_html};

pub use rich_common::parse_rich_inline_md;
pub use rich_markdown::parse_rich_markdown;

pub use rich_html::{parse_rich_html, parse_rich_html_inline};

#[cfg(test)]
mod tests {
    use super::*;
    use ferogram_tl_types as tl;

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

    // Rich message parsers

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
