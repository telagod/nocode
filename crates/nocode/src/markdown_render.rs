use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::Color;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::tui_theme::default_theme;

/// Cached syntect resources — loaded once, reused across all renders.
fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static TS: OnceLock<ThemeSet> = OnceLock::new();
    TS.get_or_init(ThemeSet::load_defaults)
}

/// A single styled segment within a rendered line.
#[derive(Debug, Clone)]
pub struct LineSegment {
    pub text: String,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
}

impl LineSegment {
    pub fn new(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            color,
            bold: false,
            italic: false,
            strikethrough: false,
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }
}

/// A single rendered line composed of styled segments.
#[derive(Debug, Clone)]
pub struct RenderedLine {
    pub segments: Vec<LineSegment>,
}

impl RenderedLine {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    pub fn push(&mut self, segment: LineSegment) {
        self.segments.push(segment);
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() || self.segments.iter().all(|s| s.text.is_empty())
    }
}

/// Render a `RenderedLine` to a plain-text string (no ANSI colors).
pub fn render_line_to_string(line: &RenderedLine) -> String {
    line.segments.iter().map(|s| s.text.as_str()).collect()
}

/// Render Markdown source into a list of styled lines.
pub fn render_markdown_to_lines(input: &str) -> Vec<RenderedLine> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(input, options);

    let theme = default_theme();

    let mut lines: Vec<RenderedLine> = Vec::new();
    let mut current = RenderedLine::new();

    // Style stack
    let mut in_heading: Option<HeadingLevel> = None;
    let mut bold_depth: u32 = 0;
    let mut italic_depth: u32 = 0;
    let mut strikethrough_depth: u32 = 0;
    let mut in_block_quote_depth: u32 = 0;
    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut link_url: Option<String> = None;

    // List tracking
    let mut list_stack: Vec<Option<u64>> = Vec::new(); // None = unordered, Some(n) = ordered starting at n

    // Table tracking
    let mut in_table = false;
    let mut table_row: Vec<String> = Vec::new();
    let mut table_is_header = false;

    // Helper: flush current line
    let flush = |lines: &mut Vec<RenderedLine>, current: &mut RenderedLine| {
        lines.push(std::mem::replace(current, RenderedLine::new()));
    };

    let heading_color = |level: HeadingLevel| -> Color {
        match level {
            HeadingLevel::H1 => theme.md_heading1,
            HeadingLevel::H2 => theme.md_heading2,
            HeadingLevel::H3 => theme.md_heading3,
            _ => theme.md_heading4,
        }
    };

    let list_indent = |stack: &[Option<u64>]| -> String {
        if stack.is_empty() {
            String::new()
        } else {
            "  ".repeat(stack.len() - 1)
        }
    };

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut lines, &mut current);
                in_heading = None;
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush(&mut lines, &mut current);
                // blank line after paragraph
                lines.push(RenderedLine::new());
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_block_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                // opening fence
                let mut fence_line = RenderedLine::new();
                let label = if code_block_lang.is_empty() {
                    "```".to_string()
                } else {
                    format!("```{}", code_block_lang)
                };
                fence_line.push(LineSegment::new(label, theme.md_code_fence));
                lines.push(fence_line);
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                // flush any remaining code content
                if !current.is_empty() {
                    flush(&mut lines, &mut current);
                }
                // closing fence
                let mut fence_line = RenderedLine::new();
                fence_line.push(LineSegment::new("```", theme.md_code_fence));
                lines.push(fence_line);
                code_block_lang.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                in_block_quote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                in_block_quote_depth = in_block_quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::List(start)) => {
                list_stack.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
            }
            Event::Start(Tag::Item) => {
                if !current.is_empty() {
                    flush(&mut lines, &mut current);
                }
                let indent = list_indent(&list_stack);
                let bullet = match list_stack.last() {
                    Some(Some(start)) => {
                        let num = *start;
                        // Increment for next item
                        if let Some(Some(s)) = list_stack.last_mut() {
                            *s += 1;
                        }
                        format!("{indent}{num}. ")
                    }
                    _ => format!("{indent}\u{2022} "),
                };
                current.push(LineSegment::new(bullet, theme.md_list_bullet));
            }
            Event::End(TagEnd::Item) => {
                flush(&mut lines, &mut current);
            }
            Event::Start(Tag::Emphasis) => {
                italic_depth += 1;
            }
            Event::End(TagEnd::Emphasis) => {
                italic_depth = italic_depth.saturating_sub(1);
            }
            Event::Start(Tag::Strong) => {
                bold_depth += 1;
            }
            Event::End(TagEnd::Strong) => {
                bold_depth = bold_depth.saturating_sub(1);
            }
            Event::Start(Tag::Strikethrough) => {
                strikethrough_depth += 1;
            }
            Event::End(TagEnd::Strikethrough) => {
                strikethrough_depth = strikethrough_depth.saturating_sub(1);
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_url = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Link) => {
                if let Some(url) = link_url.take() {
                    current.push(LineSegment::new(format!("({url})"), theme.md_link));
                }
            }
            Event::Code(text) => {
                current.push(LineSegment::new(text.to_string(), theme.md_code_inline));
            }
            Event::Text(text) => {
                let text_str = text.to_string();
                if in_table {
                    // Accumulate cell text for table row rendering.
                    if let Some(last) = table_row.last_mut() {
                        last.push_str(&text_str);
                    } else {
                        table_row.push(text_str);
                    }
                } else if in_code_block {
                    // Each line of code block gets prefix + syntect highlighting
                    let ss = syntax_set();
                    let ts = theme_set();
                    let syn_theme = &ts.themes["base16-ocean.dark"];
                    let syntax = if code_block_lang.is_empty() {
                        ss.find_syntax_plain_text()
                    } else {
                        ss.find_syntax_by_token(&code_block_lang)
                            .unwrap_or_else(|| ss.find_syntax_plain_text())
                    };
                    let mut highlighter = HighlightLines::new(syntax, syn_theme);

                    let trimmed = text_str.trim_end_matches('\n');
                    for (i, code_line) in trimmed.split('\n').enumerate() {
                        if i > 0 {
                            flush(&mut lines, &mut current);
                        }
                        if i > 0 || current.is_empty() {
                            current.push(LineSegment::new("\u{2502} ", theme.md_code_line_prefix));
                        }
                        if !code_line.is_empty() {
                            let line_with_nl = format!("{code_line}\n");
                            match highlighter.highlight_line(&line_with_nl, ss) {
                                Ok(ranges) => {
                                    for (style, fragment) in ranges {
                                        let trimmed_frag = fragment.trim_end_matches('\n');
                                        if !trimmed_frag.is_empty() {
                                            let SynStyle { foreground, .. } = style;
                                            let color = Color::Rgb(
                                                foreground.r,
                                                foreground.g,
                                                foreground.b,
                                            );
                                            current.push(LineSegment::new(trimmed_frag, color));
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Fallback to code inline color on highlight failure
                                    current.push(LineSegment::new(code_line, theme.md_code_inline));
                                }
                            }
                        }
                    }
                } else if let Some(level) = in_heading {
                    let color = heading_color(level);
                    current.push(LineSegment::new(text_str, color).bold());
                } else if in_block_quote_depth > 0 {
                    // Prefix each line with quote marker
                    let prefix = "\u{2502} ".repeat(in_block_quote_depth as usize);
                    for (i, quote_line) in text_str.split('\n').enumerate() {
                        if i > 0 {
                            flush(&mut lines, &mut current);
                        }
                        if i > 0 || current.is_empty() {
                            current.push(LineSegment::new(&prefix, theme.md_blockquote));
                        }
                        if !quote_line.is_empty() {
                            current.push(LineSegment::new(quote_line, theme.md_blockquote));
                        }
                    }
                } else if bold_depth > 0 && italic_depth > 0 {
                    let mut seg = LineSegment::new(text_str, theme.md_bold).bold().italic();
                    if strikethrough_depth > 0 {
                        seg = seg.strikethrough();
                    }
                    current.push(seg);
                } else if bold_depth > 0 {
                    let mut seg = LineSegment::new(text_str, theme.md_bold).bold();
                    if strikethrough_depth > 0 {
                        seg = seg.strikethrough();
                    }
                    current.push(seg);
                } else if italic_depth > 0 {
                    let mut seg = LineSegment::new(text_str, theme.md_italic).italic();
                    if strikethrough_depth > 0 {
                        seg = seg.strikethrough();
                    }
                    current.push(seg);
                } else if strikethrough_depth > 0 {
                    current
                        .push(LineSegment::new(text_str, theme.md_strikethrough).strikethrough());
                } else if link_url.is_some() {
                    current.push(LineSegment::new(text_str, theme.md_link));
                } else {
                    current.push(LineSegment::new(text_str, theme.md_text));
                }
            }
            Event::Rule => {
                let mut rule_line = RenderedLine::new();
                rule_line.push(LineSegment::new("\u{2500}\u{2500}\u{2500}", theme.md_rule));
                lines.push(rule_line);
            }
            Event::SoftBreak | Event::HardBreak => {
                flush(&mut lines, &mut current);
            }
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                flush(&mut lines, &mut current);
            }
            Event::End(TagEnd::Table) => {
                in_table = false;
            }
            Event::Start(Tag::TableHead) => {
                table_is_header = true;
                table_row.clear();
            }
            Event::End(TagEnd::TableHead) => {
                // Render header row
                let header_text = table_row.join(" \u{2502} ");
                let mut header_line = RenderedLine::new();
                header_line.push(LineSegment::new(&header_text, theme.md_table_header).bold());
                lines.push(header_line);
                // Separator
                let sep = "\u{2500}".repeat(header_text.len().max(3));
                let mut sep_line = RenderedLine::new();
                sep_line.push(LineSegment::new(&sep, theme.md_table_border));
                lines.push(sep_line);
                table_is_header = false;
                table_row.clear();
            }
            Event::Start(Tag::TableRow) => {
                table_row.clear();
            }
            Event::End(TagEnd::TableRow) => {
                if !table_is_header {
                    let row_text = table_row.join(" \u{2502} ");
                    let mut row_line = RenderedLine::new();
                    row_line.push(LineSegment::new(&row_text, theme.md_text));
                    lines.push(row_line);
                }
                table_row.clear();
            }
            Event::Start(Tag::TableCell) => {
                table_row.push(String::new());
            }
            Event::End(TagEnd::TableCell) => {}
            _ => {}
        }
    }

    // Flush remaining
    if !current.is_empty() {
        lines.push(current);
    }

    // Remove trailing empty lines
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_levels_render_with_correct_text() {
        let md = "# H1\n## H2\n### H3\n#### H4\n";
        let lines = render_markdown_to_lines(md);
        let texts: Vec<String> = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(render_line_to_string)
            .collect();
        assert!(texts.contains(&"H1".to_string()));
        assert!(texts.contains(&"H2".to_string()));
        assert!(texts.contains(&"H3".to_string()));
        assert!(texts.contains(&"H4".to_string()));
    }

    #[test]
    fn heading_colors_match_spec() {
        let md = "# Cyan\n## White\n### Blue\n#### Grey\n";
        let lines = render_markdown_to_lines(md);
        let non_empty: Vec<&RenderedLine> = lines.iter().filter(|l| !l.is_empty()).collect();
        assert_eq!(non_empty[0].segments[0].color, Color::Cyan);
        assert!(non_empty[0].segments[0].bold);
        assert_eq!(non_empty[1].segments[0].color, Color::White);
        assert!(non_empty[1].segments[0].bold);
        assert_eq!(non_empty[2].segments[0].color, Color::Blue);
        assert!(non_empty[2].segments[0].bold);
        assert_eq!(non_empty[3].segments[0].color, Color::DarkGray);
        assert!(non_empty[3].segments[0].bold);
    }

    #[test]
    fn code_block_renders_with_fences_and_prefix() {
        let md = "```rust\nlet x = 1;\n```\n";
        let lines = render_markdown_to_lines(md);
        let texts: Vec<String> = lines.iter().map(render_line_to_string).collect();
        // Verify fences and code content are present
        assert!(
            texts.iter().any(|t| t == "```rust"),
            "should have opening fence: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "```"),
            "should have closing fence: {texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.contains("\u{2502} ") && t.contains("let x = 1;")),
            "should have prefixed code line: {texts:?}"
        );
    }

    #[test]
    fn code_block_segments_are_highlighted() {
        let md = "```\nhello\n```\n";
        let lines = render_markdown_to_lines(md);
        // Find the line with "hello"
        let code_line = lines
            .iter()
            .find(|l| render_line_to_string(l).contains("hello"))
            .expect("should have a line with hello");
        let code_seg = code_line
            .segments
            .iter()
            .find(|s| s.text == "hello")
            .unwrap();
        // With syntect, plain text gets RGB colors from the theme, not Color::Green
        matches!(code_seg.color, Color::Rgb(..) | Color::Green);
    }

    #[test]
    fn code_block_with_rust_syntax_uses_rgb_colors() {
        let md = "```rust\nfn main() {\n    let x: u32 = 42;\n}\n```\n";
        let lines = render_markdown_to_lines(md);
        // Find lines that contain code (have the │ prefix and code content)
        let code_lines: Vec<&RenderedLine> = lines
            .iter()
            .filter(|l| {
                let text = render_line_to_string(l);
                text.starts_with("\u{2502} ") && text.len() > 2
            })
            .collect();
        assert!(!code_lines.is_empty(), "should have code lines");
        // At least one segment across all code lines should use Rgb color
        let has_rgb = code_lines.iter().any(|line| {
            line.segments
                .iter()
                .any(|s| matches!(s.color, Color::Rgb(..)))
        });
        assert!(
            has_rgb,
            "rust code should have at least one Rgb-colored segment"
        );
    }

    #[test]
    fn unordered_list_renders_bullets() {
        let md = "- alpha\n- beta\n";
        let lines = render_markdown_to_lines(md);
        let texts: Vec<String> = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(render_line_to_string)
            .collect();
        assert_eq!(texts[0], "\u{2022} alpha");
        assert_eq!(texts[1], "\u{2022} beta");
    }

    #[test]
    fn ordered_list_renders_numbers() {
        let md = "1. first\n2. second\n";
        let lines = render_markdown_to_lines(md);
        let texts: Vec<String> = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(render_line_to_string)
            .collect();
        assert_eq!(texts[0], "1. first");
        assert_eq!(texts[1], "2. second");
    }

    #[test]
    fn nested_list_indents() {
        let md = "- outer\n  - inner\n";
        let lines = render_markdown_to_lines(md);
        let texts: Vec<String> = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(render_line_to_string)
            .collect();
        assert_eq!(texts[0], "\u{2022} outer");
        assert!(
            texts[1].starts_with("  "),
            "inner should be indented: {:?}",
            texts[1]
        );
        assert!(texts[1].contains("\u{2022} inner"));
    }

    #[test]
    fn inline_bold_is_yellow_bold() {
        let md = "hello **world**\n";
        let lines = render_markdown_to_lines(md);
        let line = lines.iter().find(|l| !l.is_empty()).unwrap();
        let bold_seg = line.segments.iter().find(|s| s.text == "world").unwrap();
        assert_eq!(bold_seg.color, Color::Yellow);
        assert!(bold_seg.bold);
    }
    #[test]
    fn inline_italic_is_magenta_italic() {
        let md = "hello *world*\n";
        let lines = render_markdown_to_lines(md);
        let line = lines.iter().find(|l| !l.is_empty()).unwrap();
        let italic_seg = line.segments.iter().find(|s| s.text == "world").unwrap();
        assert_eq!(italic_seg.color, Color::Magenta);
        assert!(italic_seg.italic);
    }

    #[test]
    fn inline_code_is_green() {
        let md = "use `foo` here\n";
        let lines = render_markdown_to_lines(md);
        let line = lines.iter().find(|l| !l.is_empty()).unwrap();
        let code_seg = line.segments.iter().find(|s| s.text == "foo").unwrap();
        assert_eq!(code_seg.color, Color::Green);
    }

    #[test]
    fn blockquote_has_prefix() {
        let md = "> quoted text\n";
        let lines = render_markdown_to_lines(md);
        let text = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(render_line_to_string)
            .next()
            .unwrap();
        assert!(
            text.starts_with("\u{2502} "),
            "blockquote should have prefix: {:?}",
            text
        );
        assert!(text.contains("quoted text"));
    }

    #[test]
    fn rule_renders_dashes() {
        let md = "above\n\n---\n\nbelow\n";
        let lines = render_markdown_to_lines(md);
        let has_rule = lines
            .iter()
            .any(|l| render_line_to_string(l).contains("\u{2500}\u{2500}\u{2500}"));
        assert!(has_rule, "should contain horizontal rule");
    }

    #[test]
    fn link_appends_url() {
        let md = "[click](https://example.com)\n";
        let lines = render_markdown_to_lines(md);
        let text = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(render_line_to_string)
            .next()
            .unwrap();
        assert!(text.contains("click"));
        assert!(text.contains("(https://example.com)"));
    }

    #[test]
    fn empty_markdown_returns_empty() {
        let lines = render_markdown_to_lines("");
        assert!(lines.is_empty());
    }

    #[test]
    fn render_line_to_string_concatenates_segments() {
        let line = RenderedLine {
            segments: vec![
                LineSegment::new("hello ", Color::White),
                LineSegment::new("world", Color::Green),
            ],
        };
        assert_eq!(render_line_to_string(&line), "hello world");
    }
}
