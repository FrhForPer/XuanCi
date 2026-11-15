//! Markdown (CommonMark + GFM) to RTF renderer for the RichEdit result window.
//!
//! The renderer is pure logic and never executes HTML or JavaScript. All text
//! is escaped into RTF control sequences; remote images are never fetched.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;

use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Renders at most this many source characters.
const MAX_INPUT_CHARS: usize = 300_000;
/// Skip local images larger than this (bytes).
const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;

// Base color table indices (1-based in RTF).
const C_TEXT: usize = 1;
const C_CODE_BG: usize = 2;
const C_CODE_FG: usize = 3;
const C_MUTED: usize = 4;
const C_LINK: usize = 5;
const C_TABLE_HEAD: usize = 6;
const C_BORDER: usize = 7;
const C_HEADING: usize = 8;

const FONT_BODY: &str = "Microsoft YaHei UI";
const FONT_CODE: &str = "Consolas";
const FONT_SYMBOL: &str = "Segoe UI Symbol";

const BODY_SIZE: u32 = 21; // half-points: 10.5pt

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Embed local PNG/JPEG files referenced by absolute paths. Off by default
    /// because it lets model output read files from the local machine.
    pub embed_local_images: bool,
}

struct ListLevel {
    ordered: bool,
    next: u64,
    indent: i32,
    in_item: bool,
}

struct TableState {
    aligns: Vec<Alignment>,
    rows: Vec<TableRow>,
    current_row: Vec<String>,
    cell: String,
    in_header: bool,
    in_cell: bool,
}

struct TableRow {
    cells: Vec<String>,
    header: bool,
}

struct CodeState {
    lang: String,
    text: String,
}

struct LinkState {
    url: String,
    text: String,
}

struct ImageState {
    url: String,
    alt: String,
}

struct Rtf {
    options: RenderOptions,
    body: String,
    colors: Vec<(u8, u8, u8)>,
    color_map: HashMap<(u8, u8, u8), usize>,
    list_stack: Vec<ListLevel>,
    quote_depth: usize,
    strong_depth: usize,
    emphasis_depth: usize,
    strike_depth: usize,
    heading: Option<HeadingLevel>,
    table: Option<TableState>,
    code: Option<CodeState>,
    link: Option<LinkState>,
    image: Option<ImageState>,
    footnote_def: Option<(String, String)>,
    footnotes: Vec<(String, String)>,
    footnote_numbers: HashMap<String, usize>,
    next_footnote: usize,
}

impl Rtf {
    fn new(options: RenderOptions) -> Self {
        let mut this = Self {
            options,
            body: String::new(),
            colors: Vec::new(),
            color_map: HashMap::new(),
            list_stack: Vec::new(),
            quote_depth: 0,
            strong_depth: 0,
            emphasis_depth: 0,
            strike_depth: 0,
            heading: None,
            table: None,
            code: None,
            link: None,
            image: None,
            footnote_def: None,
            footnotes: Vec::new(),
            footnote_numbers: HashMap::new(),
            next_footnote: 0,
        };
        for rgb in [
            (33, 37, 41),    // text
            (243, 244, 246), // code background
            (36, 41, 46),    // code text
            (107, 114, 128), // muted / quote
            (37, 99, 235),   // link
            (248, 249, 250), // table header (must differ from code background)
            (209, 213, 219), // border
            (17, 24, 39),    // heading
        ] {
            this.intern(rgb);
        }
        this
    }

    fn intern(&mut self, rgb: (u8, u8, u8)) -> usize {
        if let Some(&index) = self.color_map.get(&rgb) {
            return index;
        }
        let index = self.colors.len() + 1;
        self.colors.push(rgb);
        self.color_map.insert(rgb, index);
        index
    }

    fn raw(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(table) = self.table.as_mut() {
            if table.in_cell {
                table.cell.push_str(text);
                return;
            }
        }
        self.body.push_str(text);
    }

    fn ensure_par(&mut self) {
        let has_par = if let Some(table) = &self.table {
            if table.in_cell {
                table.cell.is_empty() || ends_with_par(&table.cell)
            } else {
                ends_with_par(&self.body)
            }
        } else {
            self.body.is_empty() || ends_with_par(&self.body)
        };
        if !has_par {
            self.raw("\\par ");
        }
    }

    fn item_active(&self) -> bool {
        self.list_stack.last().map(|l| l.in_item).unwrap_or(false)
    }

    fn list_indent(&self) -> i32 {
        self.list_stack.last().map(|l| l.indent).unwrap_or(0)
    }

    fn total_indent(&self) -> i32 {
        self.list_indent() + 360 * self.quote_depth as i32
    }

    fn start(&mut self, tag: Tag<'_>) {
        if self.footnote_def.is_some() || self.image.is_some() || self.code.is_some() {
            return;
        }

        match tag {
            Tag::Paragraph => {
                if self.table.is_some() || self.item_active() {
                    return;
                }
                self.ensure_par();
                if self.quote_depth > 0 {
                    let li = 360 * self.quote_depth as i32;
                    let _ = write!(
                        self.body,
                        "\\pard\\li{li}\\fi0\\brdrl\\brdrs\\brdrw10\\brdrcf{C_BORDER}\\brsp60\\cf{C_MUTED}\\sb120\\sa120 "
                    );
                } else {
                    self.body
                        .push_str("\\pard\\li0\\fi0\\sb120\\sa120\\cf1 ");
                }
            }
            Tag::Heading { level, .. } => {
                self.ensure_par();
                let li = 360 * self.quote_depth as i32;
                let outline = heading_outline(level);
                let _ = write!(
                    self.body,
                    "\\pard\\li{li}\\fi0\\sb240\\sa120\\outlinelevel{outline}\\keepn\\cf{C_HEADING} "
                );
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some(CodeState {
                    lang,
                    text: String::new(),
                });
            }
            Tag::HtmlBlock => {
                self.ensure_par();
                self.body
                    .push_str("\\pard\\li0\\fi0\\sb120\\sa120\\cf1 ");
            }
            Tag::List(start) => {
                self.list_stack.push(ListLevel {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                    indent: 360 * (self.list_stack.len() as i32 + 1),
                    in_item: false,
                });
            }
            Tag::Item => self.start_item(),
            Tag::FootnoteDefinition(label) => {
                self.footnote_def = Some((label.to_string(), String::new()));
            }
            Tag::Table(aligns) => {
                self.ensure_par();
                self.table = Some(TableState {
                    aligns,
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    cell: String::new(),
                    in_header: false,
                    in_cell: false,
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.clear();
                    table.in_header = true;
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.cell.clear();
                    table.in_cell = true;
                }
            }
            Tag::Emphasis => self.emphasis_depth += 1,
            Tag::Strong => self.strong_depth += 1,
            Tag::Strikethrough => self.strike_depth += 1,
            Tag::Link { dest_url, .. } => {
                self.link = Some(LinkState {
                    url: dest_url.to_string(),
                    text: String::new(),
                });
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some(ImageState {
                    url: dest_url.to_string(),
                    alt: String::new(),
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        if self.footnote_def.is_some() {
            if tag == TagEnd::FootnoteDefinition {
                self.finish_footnote();
            }
            return;
        }
        if self.image.is_some() {
            if tag == TagEnd::Image {
                self.finish_image();
            }
            return;
        }
        if self.code.is_some() {
            if tag == TagEnd::CodeBlock {
                self.finish_code();
            }
            return;
        }

        match tag {
            TagEnd::Paragraph => {
                if self.table.is_none() {
                    self.ensure_par();
                }
            }
            TagEnd::Heading(_) => {
                self.heading = None;
                self.ensure_par();
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.ensure_par();
                if self.quote_depth == 0 {
                    self.body
                        .push_str("\\pard\\li0\\fi0\\sb120\\sa120\\cf1 ");
                }
            }
            TagEnd::CodeBlock => self.finish_code(),
            TagEnd::HtmlBlock => {
                self.ensure_par();
            }
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => self.end_item(),
            TagEnd::FootnoteDefinition => self.finish_footnote(),
            TagEnd::Table => self.finish_table(),
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    commit_table_row(table, true);
                    table.in_header = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    commit_table_row(table, false);
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.in_cell = false;
                    let cell = std::mem::take(&mut table.cell);
                    table.current_row.push(cell);
                }
            }
            TagEnd::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_sub(1),
            TagEnd::Strong => self.strong_depth = self.strong_depth.saturating_sub(1),
            TagEnd::Strikethrough => self.strike_depth = self.strike_depth.saturating_sub(1),
            TagEnd::Link => self.finish_link(),
            TagEnd::Image => self.finish_image(),
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, buffer)) = self.footnote_def.as_mut() {
            buffer.push_str(text);
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(text);
            return;
        }
        if let Some(code) = self.code.as_mut() {
            code.text.push_str(text);
            return;
        }
        if let Some(link) = self.link.as_mut() {
            link.text.push_str(text);
        }
        self.push_styled_text(text);
    }

    fn code(&mut self, code: &str) {
        if let Some((_, buffer)) = self.footnote_def.as_mut() {
            buffer.push_str(code);
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(code);
            return;
        }
        if let Some(link) = self.link.as_mut() {
            link.text.push_str(code);
        }
        let mut run = String::from("{\\f1\\fs18\\cb2\\cf3\\~");
        escape_into(code, &mut run);
        run.push_str("\\~}");
        self.raw(&run);
    }

    fn math(&mut self, math: &str, display: bool) {
        if let Some((_, buffer)) = self.footnote_def.as_mut() {
            buffer.push_str(math);
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(math);
            return;
        }
        if let Some(link) = self.link.as_mut() {
            link.text.push_str(math);
        }
        let mut run = String::from("{\\f1\\fs18\\cf3 ");
        if display {
            run.push_str("$$");
        } else {
            run.push('$');
        }
        escape_into(math, &mut run);
        if display {
            run.push_str("$$");
        } else {
            run.push('$');
        }
        run.push('}');
        self.raw(&run);
    }

    fn html(&mut self, html: &str) {
        if let Some((_, buffer)) = self.footnote_def.as_mut() {
            buffer.push_str(html);
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(html);
            return;
        }
        if let Some(link) = self.link.as_mut() {
            link.text.push_str(html);
        }
        let mut run = String::from("{\\f1\\fs18\\cf4 ");
        escape_into(html, &mut run);
        run.push('}');
        self.raw(&run);
    }

    fn soft_break(&mut self) {
        if let Some((_, buffer)) = self.footnote_def.as_mut() {
            buffer.push(' ');
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push(' ');
            return;
        }
        if let Some(link) = self.link.as_mut() {
            link.text.push(' ');
        }
        self.raw(" ");
    }

    fn hard_break(&mut self) {
        if let Some((_, buffer)) = self.footnote_def.as_mut() {
            buffer.push('\n');
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push(' ');
            return;
        }
        if let Some(link) = self.link.as_mut() {
            link.text.push(' ');
        }
        self.raw("\\line ");
    }

    fn rule(&mut self) {
        if self.table.is_some() || self.footnote_def.is_some() || self.image.is_some() {
            return;
        }
        self.ensure_par();
        let _ = write!(
            self.body,
            "\\pard\\li0\\fi0\\brdrb\\brdrs\\brdrw10\\brdrcf{C_BORDER}\\sb120\\sa120\\cf1\\par "
        );
    }

    fn task_marker(&mut self, checked: bool) {
        if self.footnote_def.is_some() || self.image.is_some() {
            return;
        }
        // U+2611 / U+2610, rendered through the symbol font.
        let symbol = if checked { "\\u9745? " } else { "\\u9744? " };
        self.raw(&format!("{{\\f2 {symbol}}}"));
    }

    fn footnote_ref(&mut self, label: &str) {
        if self.footnote_def.is_some() || self.image.is_some() {
            return;
        }
        let number = if let Some(&number) = self.footnote_numbers.get(label) {
            number
        } else {
            self.next_footnote += 1;
            let number = self.next_footnote;
            self.footnote_numbers.insert(label.to_string(), number);
            number
        };
        let run = format!("{{\\super\\fs15\\cf{C_LINK} [{number}]}}");
        self.raw(&run);
    }

    fn push_styled_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let in_header = self
            .table
            .as_ref()
            .map(|table| table.in_header && table.in_cell)
            .unwrap_or(false);

        let mut style = String::new();
        if let Some(level) = self.heading {
            let _ = write!(style, "\\fs{}\\cf{C_HEADING}", heading_size(level));
        } else if self.quote_depth > 0 {
            let _ = write!(style, "\\cf{C_MUTED}");
        } else {
            let _ = write!(style, "\\cf{C_TEXT}");
        }

        if self.heading.is_some() || self.strong_depth > 0 || in_header {
            style.push_str("\\b");
        }
        if self.emphasis_depth > 0 {
            style.push_str("\\i");
        }
        if self.strike_depth > 0 {
            style.push_str("\\strike");
        }
        if self.link.is_some() {
            let _ = write!(style, "\\ul\\cf{C_LINK}");
        }

        let mut run = String::from("{");
        run.push_str(&style);
        run.push(' ');
        escape_into(text, &mut run);
        run.push('}');
        self.raw(&run);
    }

    fn start_item(&mut self) {
        self.ensure_par();
        let Some(level) = self.list_stack.last_mut() else {
            return;
        };
        level.in_item = true;
        let indent = level.indent;
        let ordered = level.ordered;
        let number = level.next;

        let li = indent + 360 * self.quote_depth as i32;
        let color = if self.quote_depth > 0 { C_MUTED } else { C_TEXT };
        let _ = write!(
            self.body,
            "\\pard\\li{li}\\fi-360\\sb20\\sa20\\cf{color} "
        );
        if ordered {
            let _ = write!(self.body, "{number}. ");
        } else {
            self.body.push_str("\\u8226? ");
        }
    }

    fn end_item(&mut self) {
        self.ensure_par();
        if let Some(level) = self.list_stack.last_mut() {
            level.in_item = false;
            if level.ordered {
                level.next += 1;
            }
        }
    }

    fn end_list(&mut self) {
        self.ensure_par();
        self.list_stack.pop();
        let quote = self.quote_depth as i32;
        if let Some(level) = self.list_stack.last() {
            let li = level.indent + 360 * quote;
            let color = if quote > 0 { C_MUTED } else { C_TEXT };
            let _ = write!(
                self.body,
                "\\pard\\li{li}\\fi0\\sb0\\sa0\\cf{color} "
            );
        } else {
            self.body
                .push_str("\\pard\\li0\\fi0\\sb120\\sa120\\cf1 ");
        }
    }

    fn finish_link(&mut self) {
        let Some(link) = self.link.take() else {
            return;
        };
        let text = link.text.trim();
        if link.url.is_empty() {
            return;
        }
        if text == link.url && !text.is_empty() {
            return;
        }
        let mut run = format!("{{\\fs16\\cf{C_MUTED} (");
        escape_into(&link.url, &mut run);
        run.push_str(")}");
        self.raw(&run);
    }

    fn finish_image(&mut self) {
        let Some(image) = self.image.take() else {
            return;
        };
        if self.options.embed_local_images {
            if let Some(picture) = load_local_image(&image.url) {
                self.ensure_par();
                self.raw(&picture);
                self.raw("\\par ");
                return;
            }
        }

        let label = if image.alt.trim().is_empty() {
            "图片"
        } else {
            image.alt.trim()
        };
        let mut run = format!("{{\\cf{C_MUTED} [");
        escape_into(label, &mut run);
        run.push(']');
        if !image.url.is_empty() && image.url != label {
            let _ = write!(run, "{{\\fs16\\cf{C_MUTED} ");
            escape_into(&image.url, &mut run);
            run.push('}');
        }
        run.push('}');
        self.raw(&run);
    }

    fn finish_footnote(&mut self) {
        if let Some((label, text)) = self.footnote_def.take() {
            self.footnotes.push((label, text.trim().to_string()));
        }
    }

    fn finish_code(&mut self) {
        let Some(code) = self.code.take() else {
            return;
        };
        self.ensure_par();
        let li = self.total_indent();
        let runs = self.code_runs(&code.text, &code.lang);
        let _ = write!(
            self.body,
            "\\pard\\li{li}\\fi0\\sb120\\sa120\\cf{C_TEXT}{{\\f1\\fs18\\cb{C_CODE_BG}\\cf{C_CODE_FG}\\~"
        );
        self.body.push_str(&runs);
        self.body.push_str("\\~}\\par ");
    }

    fn code_runs(&mut self, code: &str, lang: &str) -> String {
        let lang = lang
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or("");
        let syntax = if lang.is_empty() {
            None
        } else {
            syntaxes()
                .find_syntax_by_token(lang)
                .or_else(|| syntaxes().find_syntax_by_extension(lang))
        };
        let Some(syntax) = syntax else {
            return plain_code_runs(code);
        };

        let lines: Vec<&str> = LinesWithEndings::from(code).collect();
        if lines.is_empty() {
            return String::new();
        }
        let mut highlighter = HighlightLines::new(syntax, theme());
        let mut out = String::new();

        for (index, line) in lines.iter().enumerate() {
            let ranges = match highlighter.highlight_line(line, syntaxes()) {
                Ok(ranges) => ranges,
                Err(_) => return plain_code_runs(code),
            };
            for (style, text) in ranges {
                let text = text.trim_end_matches(['\r', '\n']);
                if text.is_empty() {
                    continue;
                }
                let color = self.intern((
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                ));
                let mut run = format!("{{\\cf{color}");
                if style.font_style.contains(FontStyle::BOLD) {
                    run.push_str("\\b");
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    run.push_str("\\i");
                }
                if style.font_style.contains(FontStyle::UNDERLINE) {
                    run.push_str("\\ul");
                }
                run.push(' ');
                escape_into(text, &mut run);
                run.push('}');
                out.push_str(&run);
            }
            if index + 1 < lines.len() {
                out.push_str("\\line ");
            }
        }
        out
    }

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        if !self.body.is_empty() && !ends_with_par(&self.body) {
            self.body.push_str("\\par ");
        }

        let cols = table
            .aligns
            .len()
            .max(table.rows.iter().map(|row| row.cells.len()).max().unwrap_or(0))
            .max(1);
        let cell_width = 9000 / cols as i32;

        for row in &table.rows {
            let mut row_def = String::from("\\trowd\\trgaph80\\trleft0");
            for column in 0..cols {
                row_def.push_str(
                    "\\clvertalt\\clbrdrt\\brdrs\\brdrw8\\brdrcf7\
                     \\clbrdrl\\brdrs\\brdrw8\\brdrcf7\
                     \\clbrdrb\\brdrs\\brdrw8\\brdrcf7\
                     \\clbrdrr\\brdrs\\brdrw8\\brdrcf7",
                );
                if row.header {
                    let _ = write!(row_def, "\\clcbpat{C_TABLE_HEAD}");
                }
                let _ = write!(row_def, "\\cellx{}", (column as i32 + 1) * cell_width);
            }
            self.body.push_str(&row_def);

            for column in 0..cols {
                let align = table.aligns.get(column).copied().unwrap_or(Alignment::None);
                let align_ctrl = match align {
                    Alignment::Center => "\\qc",
                    Alignment::Right => "\\qr",
                    _ => "\\ql",
                };
                let cell = row.cells.get(column).map(String::as_str).unwrap_or("");
                let _ = write!(
                    self.body,
                    "\\pard\\intbl\\sb20\\sa20\\cf{C_TEXT}{align_ctrl} "
                );
                if row.header {
                    self.body.push_str("{\\b ");
                }
                self.body.push_str(cell);
                if row.header {
                    self.body.push_str("}");
                }
                self.body.push_str("\\cell");
            }
            self.body.push_str("\\row ");
        }
        self.body
            .push_str("\\pard\\sb120\\sa120\\cf1 ");
    }

    fn finish_footnotes(&mut self) {
        if self.footnotes.is_empty() {
            return;
        }
        // Assign numbers to definitions that were never referenced.
        for (label, _) in &self.footnotes {
            if !self.footnote_numbers.contains_key(label) {
                self.next_footnote += 1;
                let number = self.next_footnote;
                self.footnote_numbers.insert(label.clone(), number);
            }
        }

        let mut items: Vec<(usize, String)> = self
            .footnotes
            .iter()
            .map(|(label, text)| {
                (
                    self.footnote_numbers.get(label).copied().unwrap_or(0),
                    text.clone(),
                )
            })
            .collect();
        items.sort_by_key(|(number, _)| *number);

        self.ensure_par();
        self.body.push_str(
            "\\pard\\brdrt\\brdrs\\brdrw10\\brdrcf7\\sb240\\sa120{\\b\\fs18 ",
        );
        escape_into("脚注", &mut self.body);
        self.body.push_str("}\\par ");
        for (number, text) in items {
            let _ = write!(
                self.body,
                "\\pard\\li360\\fi-360\\sb20\\sa20{{\\fs18\\cf{C_MUTED} [{number}] "
            );
            escape_into(&text, &mut self.body);
            self.body.push_str("\\par ");
        }
    }

    fn finish(mut self) -> String {
        self.finish_footnotes();
        self.ensure_par();

        let mut out = String::with_capacity(self.body.len() + 1024);
        out.push_str("{\\rtf1\\ansi\\ansicpg65001\\deff0\\uc1\n");
        let _ = write!(
            out,
            "{{\\fonttbl{{\\f0\\fnil\\fcharset134 {FONT_BODY};}}{{\\f1\\fmodern\\fcharset0 {FONT_CODE};}}{{\\f2\\fnil\\fcharset0 {FONT_SYMBOL};}}}}\n"
        );
        out.push_str("{\\colortbl;");
        for (r, g, b) in &self.colors {
            let _ = write!(out, "\\red{r}\\green{g}\\blue{b};");
        }
        out.push_str("}\n");
        let _ = write!(
            out,
            "\\viewkind4\\pard\\f0\\fs{BODY_SIZE}\\sl276\\slmult1\\cf{C_TEXT}\n"
        );
        out.push_str(&self.body);
        out.push_str("\n}\n");
        out
    }
}

fn ends_with_par(text: &str) -> bool {
    text.trim_end().ends_with("\\par")
}

fn commit_table_row(table: &mut TableState, header: bool) {
    if table.current_row.is_empty() {
        return;
    }
    let mut cells = std::mem::take(&mut table.current_row);
    cells.resize(table.aligns.len().max(cells.len()), String::new());
    table.rows.push(TableRow { cells, header });
}

fn heading_size(level: HeadingLevel) -> u32 {
    match level {
        HeadingLevel::H1 => 36,
        HeadingLevel::H2 => 32,
        HeadingLevel::H3 => 28,
        HeadingLevel::H4 => 24,
        HeadingLevel::H5 => 22,
        HeadingLevel::H6 => 21,
    }
}

fn heading_outline(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 0,
        HeadingLevel::H2 => 1,
        HeadingLevel::H3 => 2,
        HeadingLevel::H4 => 3,
        HeadingLevel::H5 => 4,
        HeadingLevel::H6 => 5,
    }
}

fn escape_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\line "),
            '\r' => {}
            '\t' => out.push_str("\\tab "),
            ch if (ch as u32) < 0x20 => {}
            ch if (ch as u32) < 0x80 => out.push(ch),
            ch => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    let signed = *unit as i16 as i32;
                    let _ = write!(out, "\\u{signed}?");
                }
            }
        }
    }
}

fn options_flags() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH
}

/// Converts Markdown to RTF suitable for `EM_STREAMIN(SF_RTF)`.
pub fn to_rtf(markdown: &str) -> String {
    to_rtf_with(markdown, RenderOptions::default())
}

pub fn to_rtf_with(markdown: &str, options: RenderOptions) -> String {
    let input = if markdown.chars().count() > MAX_INPUT_CHARS {
        markdown.chars().take(MAX_INPUT_CHARS).collect::<String>()
    } else {
        markdown.to_string()
    };
    let mut rtf = Rtf::new(options);
    for event in Parser::new_ext(&input, options_flags()) {
        match event {
            Event::Start(tag) => rtf.start(tag),
            Event::End(tag) => rtf.end(tag),
            Event::Text(text) => rtf.text(&text),
            Event::Code(code) => rtf.code(&code),
            Event::InlineMath(math) => rtf.math(&math, false),
            Event::DisplayMath(math) => rtf.math(&math, true),
            Event::Html(html) | Event::InlineHtml(html) => rtf.html(&html),
            Event::FootnoteReference(label) => rtf.footnote_ref(&label),
            Event::SoftBreak => rtf.soft_break(),
            Event::HardBreak => rtf.hard_break(),
            Event::Rule => rtf.rule(),
            Event::TaskListMarker(checked) => rtf.task_marker(checked),
        }
    }
    rtf.finish()
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
    let set = THEME_SET.get_or_init(ThemeSet::load_defaults);
    set.themes
        .get("InspiredGitHub")
        .or_else(|| set.themes.get("base16-ocean.light"))
        .or_else(|| set.themes.values().next())
        .expect("syntect ships default themes")
}

fn plain_code_runs(code: &str) -> String {
    let mut out = String::new();
    for (index, line) in code.trim_end_matches('\n').split('\n').enumerate() {
        if index > 0 {
            out.push_str("\\line ");
        }
        escape_into(line, &mut out);
    }
    out
}

/// Embeds a local PNG/JPEG as an RTF picture. Remote URLs are never fetched.
fn load_local_image(url: &str) -> Option<String> {
    let path = local_image_path(url)?;
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;

    let (blip, width, height) = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        if bytes.len() < 24 {
            return None;
        }
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        ("\\pngblip", width, height)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let (width, height) = jpeg_dimensions(&bytes)?;
        ("\\jpegblip", width, height)
    } else {
        return None;
    };
    if width == 0 || height == 0 {
        return None;
    }

    let mut goal_w = width as i64 * 15;
    let mut goal_h = height as i64 * 15;
    let max_w = 6000i64;
    let max_h = 4000i64;
    if goal_w > max_w || goal_h > max_h {
        let scale_w = max_w as f64 / goal_w as f64;
        let scale_h = max_h as f64 / goal_h as f64;
        let scale = scale_w.min(scale_h);
        goal_w = (goal_w as f64 * scale).max(1.0) as i64;
        goal_h = (goal_h as f64 * scale).max(1.0) as i64;
    }

    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in &bytes {
        let _ = write!(hex, "{byte:02x}");
    }

    Some(format!(
        "{{\\pict{blip}\\picw{width}\\pich{height}\\picwgoal{goal_w}\\pichgoal{goal_h} {hex}}}"
    ))
}

fn local_image_path(url: &str) -> Option<PathBuf> {
    let url = url.trim();
    let extension_ok = {
        let lower = url.to_ascii_lowercase();
        lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.contains(".png?")
            || lower.contains(".jpg?")
            || lower.contains(".jpeg?")
    };
    if !extension_ok {
        return None;
    }

    if let Some(rest) = url.strip_prefix("file:///") {
        let path = rest.replace('/', "\\");
        return Some(PathBuf::from(path));
    }
    if let Some(rest) = url.strip_prefix("file://") {
        let path = rest.replace('/', "\\");
        return Some(PathBuf::from(path));
    }

    let bytes = url.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return Some(PathBuf::from(url));
    }
    None
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2;
    while index + 9 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        if marker == 0xD8 || marker == 0xD9 {
            index += 2;
            continue;
        }
        let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
        if length < 2 || index + 2 + length > bytes.len() {
            break;
        }
        let is_sof = matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        );
        if is_sof {
            let height = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]) as u32;
            return Some((width, height));
        }
        index += 2 + length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings_and_emphasis() {
        let rtf = to_rtf("# 标题\n\n**粗体** 和 *斜体*，还有 ~~删除线~~。");
        assert!(rtf.is_ascii());
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.contains("\\fs36"));
        assert!(rtf.contains("\\b"));
        assert!(rtf.contains("\\i"));
        assert!(rtf.contains("\\strike"));
        assert!(rtf.contains("\\u26631?")); // 标
        assert!(rtf.ends_with("}\n"));
    }

    #[test]
    fn escapes_rtf_control_characters() {
        let rtf = to_rtf(r"literal \ { } and <script>alert(1)</script>");
        assert!(rtf.contains("literal \\\\ \\{ \\}"));
        assert!(rtf.contains("<script>"));
        assert!(!rtf.contains("\\pard{\\*"));
    }

    #[test]
    fn renders_tables_and_tasks() {
        let rtf = to_rtf(
            "| A | B |\n|---|---:|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n",
        );
        assert!(rtf.contains("\\trowd"));
        assert!(rtf.contains("\\cellx"));
        assert!(rtf.contains("\\clcbpat6")); // header background
        assert!(rtf.contains("{\\b "));     // header text
        assert!(rtf.contains("\\u9745?"));  // checked box
        assert!(rtf.contains("\\u9744?")); // unchecked box
    }

    #[test]
    fn renders_code_and_links() {
        let rtf = to_rtf(
            "```rust\nfn main() { println!(\"hi\"); }\n```\n\n[文档](https://example.com)",
        );
        assert!(rtf.contains("\\f1\\fs18\\cb"));
        assert!(rtf.contains("https://example.com"));
        assert!(rtf.contains("\\cf5"));
    }

    #[test]
    fn renders_footnotes() {
        let rtf = to_rtf("正文[^1]。\n\n[^1]: 脚注内容\n");
        assert!(rtf.is_ascii());
        assert!(rtf.contains("\\super"));
        assert!(rtf.contains("\\u-32486?")); // 脚 (signed 16-bit form)
        assert!(rtf.contains("\\u27880?"));  // 注
        assert!(!rtf.contains("脚注"));
    }

    #[test]
    fn embeds_local_png_when_enabled() {
        // 1x1 transparent PNG.
        let hex = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489\
                   0000000a49444154789c6360000100000500010d0a2db40000000049454e44ae426082";
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect();
        let path = std::env::temp_dir().join(format!(
            "xuanci-md-test-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).unwrap();
        let markdown = format!("![测试]({})", path.display());
        let rtf = to_rtf_with(
            &markdown,
            RenderOptions {
                embed_local_images: true,
            },
        );
        let _ = std::fs::remove_file(&path);

        assert!(rtf.is_ascii());
        assert!(rtf.contains("\\pict\\pngblip"));
        assert!(rtf.contains("\\picwgoal"));
    }

    #[test]
    fn remote_images_are_placeholders() {
        let rtf = to_rtf("![图](https://example.com/a.png)");
        assert!(rtf.contains("["));
        assert!(rtf.contains("https://example.com/a.png"));
        assert!(!rtf.contains("\\pict"));
    }

    #[test]
    fn plain_text_has_no_markdown_markers() {
        let rtf = to_rtf("**hello** `code`");
        assert!(rtf.contains("hello"));
        assert!(!rtf.contains("**"));
        assert!(rtf.contains("\\~code\\~"));
    }
}
