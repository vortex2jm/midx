use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::domain::models::TextChunk;

/// Default target maximum word count per semantic chunk (~450-500 words).
pub const DEFAULT_MAX_CHUNK_WORDS: usize = 480;

#[derive(Debug, Clone)]
struct LogicalBlock {
    text: String,
    line_start: usize,
    word_count: usize,
}

/// Pure AST-based Markdown chunker powered by `pulldown-cmark` (zero I/O).
#[derive(Debug, Clone)]
pub struct MarkdownChunker {
    max_words: usize,
}

impl Default for MarkdownChunker {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CHUNK_WORDS)
    }
}

impl MarkdownChunker {
    /// Creates a new `MarkdownChunker` with a soft per-chunk word budget.
    pub fn new(max_words: usize) -> Self {
        Self {
            max_words: max_words.max(1),
        }
    }

    /// Parses a Markdown document and splits it into semantic chunks with heading trails.
    pub fn chunk(&self, markdown: &str) -> Vec<TextChunk> {
        if markdown.trim().is_empty() {
            return Vec::new();
        }

        let line_starts = compute_line_starts(markdown);
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS;

        let parser = Parser::new_ext(markdown, options).into_offset_iter();

        let mut heading_stack: Vec<(u8, String)> = Vec::new();
        let mut active_heading_level: Option<u8> = None;
        let mut heading_buf = String::new();

        let mut section_blocks: Vec<LogicalBlock> = Vec::new();
        let mut block_buf = String::new();
        let mut block_line_start: Option<usize> = None;
        let mut in_table_cell = false;

        let mut chunks: Vec<TextChunk> = Vec::new();

        for (event, range) in parser {
            let event_line = line_of_offset(&line_starts, range.start);

            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    flush_block(&mut block_buf, &mut block_line_start, &mut section_blocks);
                    let current_ctx = format_heading_context(&heading_stack);
                    self.flush_section(&mut section_blocks, &current_ctx, &mut chunks);

                    active_heading_level = Some(heading_level_num(level));
                    heading_buf.clear();
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(level) = active_heading_level.take() {
                        let title = heading_buf.split_whitespace().collect::<Vec<_>>().join(" ");
                        while let Some((top_level, _)) = heading_stack.last() {
                            if *top_level >= level {
                                heading_stack.pop();
                            } else {
                                break;
                            }
                        }
                        if !title.is_empty() {
                            heading_stack.push((level, title));
                        }
                        heading_buf.clear();
                    }
                }
                _ if active_heading_level.is_some() => match event {
                    Event::Text(text) | Event::Code(text) => heading_buf.push_str(&text),
                    Event::SoftBreak | Event::HardBreak => heading_buf.push(' '),
                    _ => {}
                },
                Event::Start(
                    Tag::Paragraph
                    | Tag::CodeBlock(_)
                    | Tag::Item
                    | Tag::BlockQuote(_)
                    | Tag::TableRow
                    | Tag::HtmlBlock,
                ) => {
                    flush_block(&mut block_buf, &mut block_line_start, &mut section_blocks);
                    block_line_start = Some(event_line);
                }
                Event::Start(Tag::TableCell) => {
                    in_table_cell = true;
                    block_line_start.get_or_insert(event_line);
                }
                Event::End(TagEnd::TableCell) => {
                    if in_table_cell {
                        in_table_cell = false;
                        if !block_buf.ends_with(' ') && !block_buf.is_empty() {
                            block_buf.push_str(" | ");
                        }
                    }
                }
                Event::End(
                    TagEnd::Paragraph
                    | TagEnd::CodeBlock
                    | TagEnd::Item
                    | TagEnd::BlockQuote(_)
                    | TagEnd::TableRow
                    | TagEnd::HtmlBlock,
                ) => {
                    flush_block(&mut block_buf, &mut block_line_start, &mut section_blocks);
                }
                Event::TaskListMarker(checked) => {
                    block_line_start.get_or_insert(event_line);
                    block_buf.push_str(if checked { "[x] " } else { "[ ] " });
                }
                Event::Text(text)
                | Event::Code(text)
                | Event::Html(text)
                | Event::InlineHtml(text) => {
                    block_line_start.get_or_insert(event_line);
                    block_buf.push_str(&text);
                }
                Event::SoftBreak | Event::HardBreak
                    if block_line_start.is_some() && !block_buf.ends_with('\n') =>
                {
                    block_buf.push('\n');
                }
                _ => {}
            }
        }

        flush_block(&mut block_buf, &mut block_line_start, &mut section_blocks);
        let final_ctx = format_heading_context(&heading_stack);
        self.flush_section(&mut section_blocks, &final_ctx, &mut chunks);

        chunks
    }

    fn flush_section(
        &self,
        blocks: &mut Vec<LogicalBlock>,
        heading_context: &str,
        out_chunks: &mut Vec<TextChunk>,
    ) {
        if blocks.is_empty() {
            return;
        }

        let mut chunk_text = String::new();
        let mut chunk_words = 0usize;
        let mut chunk_line_start: Option<usize> = None;

        for block in blocks.drain(..) {
            if block.word_count > self.max_words {
                if !chunk_text.trim().is_empty() {
                    out_chunks.push(TextChunk {
                        heading_context: heading_context.to_string(),
                        content: chunk_text.trim().to_string(),
                        line_start: chunk_line_start.unwrap_or(1),
                    });
                    chunk_text.clear();
                    chunk_words = 0;
                    chunk_line_start = None;
                }

                self.split_oversized_block(&block, heading_context, out_chunks);
                continue;
            }

            if chunk_words > 0 && chunk_words + block.word_count > self.max_words {
                out_chunks.push(TextChunk {
                    heading_context: heading_context.to_string(),
                    content: chunk_text.trim().to_string(),
                    line_start: chunk_line_start.unwrap_or(block.line_start),
                });
                chunk_text.clear();
                chunk_words = 0;
                chunk_line_start = None;
            }

            chunk_line_start.get_or_insert(block.line_start);
            if !chunk_text.is_empty() {
                chunk_text.push_str("\n\n");
            }
            chunk_text.push_str(&block.text);
            chunk_words += block.word_count;
        }

        if !chunk_text.trim().is_empty() {
            out_chunks.push(TextChunk {
                heading_context: heading_context.to_string(),
                content: chunk_text.trim().to_string(),
                line_start: chunk_line_start.unwrap_or(1),
            });
        }
    }

    fn split_oversized_block(
        &self,
        block: &LogicalBlock,
        heading_context: &str,
        out_chunks: &mut Vec<TextChunk>,
    ) {
        let mut current = String::new();
        let mut current_words = 0usize;
        let mut current_line = block.line_start;

        for (line_cursor, line) in (block.line_start..).zip(block.text.lines()) {
            let line_words: Vec<&str> = line.split_whitespace().collect();
            if line_words.len() > self.max_words {
                if !current.trim().is_empty() {
                    out_chunks.push(TextChunk {
                        heading_context: heading_context.to_string(),
                        content: current.trim().to_string(),
                        line_start: current_line,
                    });
                    current.clear();
                    current_words = 0;
                }

                for word_slice in line_words.chunks(self.max_words) {
                    out_chunks.push(TextChunk {
                        heading_context: heading_context.to_string(),
                        content: word_slice.join(" "),
                        line_start: line_cursor,
                    });
                }
                current_line = line_cursor + 1;
            } else {
                if current_words > 0 && current_words + line_words.len() > self.max_words {
                    out_chunks.push(TextChunk {
                        heading_context: heading_context.to_string(),
                        content: current.trim().to_string(),
                        line_start: current_line,
                    });
                    current.clear();
                    current_words = 0;
                    current_line = line_cursor;
                }

                if current.is_empty() {
                    current_line = line_cursor;
                } else {
                    current.push('\n');
                }
                current.push_str(line);
                current_words += line_words.len();
            }
        }

        if !current.trim().is_empty() {
            out_chunks.push(TextChunk {
                heading_context: heading_context.to_string(),
                content: current.trim().to_string(),
                line_start: current_line,
            });
        }
    }
}

fn flush_block(
    block_buf: &mut String,
    block_line_start: &mut Option<usize>,
    section_blocks: &mut Vec<LogicalBlock>,
) {
    let trimmed = block_buf.trim().trim_end_matches(" |").trim();
    if !trimmed.is_empty() {
        let word_count = trimmed.split_whitespace().count().max(1);
        section_blocks.push(LogicalBlock {
            text: trimmed.to_string(),
            line_start: block_line_start.unwrap_or(1),
            word_count,
        });
    }
    block_buf.clear();
    *block_line_start = None;
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_of_offset(line_starts: &[usize], byte_offset: usize) -> usize {
    line_starts
        .partition_point(|&start| start <= byte_offset)
        .max(1)
}

fn heading_level_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn format_heading_context(stack: &[(u8, String)]) -> String {
    stack
        .iter()
        .map(|(level, title)| format!("{} {}", "#".repeat(*level as usize), title))
        .collect::<Vec<_>>()
        .join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_heading_hierarchy_and_line_starts() {
        let md = "\
# Arquitetura
Introducao na linha 2.

## Ports & Adapters
Detalhes da porta na linha 5.

### NoteRepository
Persistencia SQLite na linha 8.

## Pipeline
Nova secao H2 limpa o H3.
";
        let chunker = MarkdownChunker::default();
        let chunks = chunker.chunk(md);

        assert_eq!(chunks.len(), 4);

        assert_eq!(chunks[0].heading_context, "# Arquitetura");
        assert_eq!(chunks[0].line_start, 2);
        assert!(chunks[0].content.contains("Introducao na linha 2."));

        assert_eq!(
            chunks[1].heading_context,
            "# Arquitetura > ## Ports & Adapters"
        );
        assert_eq!(chunks[1].line_start, 5);
        assert!(chunks[1].content.contains("Detalhes da porta na linha 5."));

        assert_eq!(
            chunks[2].heading_context,
            "# Arquitetura > ## Ports & Adapters > ### NoteRepository"
        );
        assert_eq!(chunks[2].line_start, 8);
        assert!(
            chunks[2]
                .content
                .contains("Persistencia SQLite na linha 8.")
        );

        assert_eq!(chunks[3].heading_context, "# Arquitetura > ## Pipeline");
        assert_eq!(chunks[3].line_start, 11);
        assert!(chunks[3].content.contains("Nova secao H2 limpa o H3."));
    }

    #[test]
    fn ignores_hash_comments_inside_code_blocks() {
        let md = "\
# Guide

```bash
# Not a markdown heading
echo hello
```
";
        let chunker = MarkdownChunker::default();
        let chunks = chunker.chunk(md);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading_context, "# Guide");
        assert_eq!(chunks[0].line_start, 3);
        assert!(chunks[0].content.contains("# Not a markdown heading"));
    }

    #[test]
    fn splits_long_sections_preserving_heading_context() {
        let mut md = String::from("# Long Section\n\n");
        let paragraph = (0..180)
            .map(|i| format!("palavra{i}"))
            .collect::<Vec<_>>()
            .join(" ");

        // Three paragraphs of 180 words each = 540 words total (> 480)
        md.push_str(&paragraph);
        md.push_str("\n\n");
        md.push_str(&paragraph);
        md.push_str("\n\n");
        md.push_str(&paragraph);
        md.push('\n');

        let chunker = MarkdownChunker::new(480);
        let chunks = chunker.chunk(&md);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading_context, "# Long Section");
        assert_eq!(chunks[1].heading_context, "# Long Section");
        assert_eq!(chunks[0].line_start, 3);
        assert_eq!(chunks[1].line_start, 7);
    }

    #[test]
    fn handles_preamble_before_first_heading_and_empty_input() {
        let chunker = MarkdownChunker::default();
        assert!(chunker.chunk("   \n\n  ").is_empty());

        let md = "Preamble text.\n\n# First\nBody text.\n";
        let chunks = chunker.chunk(md);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading_context, "");
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[1].heading_context, "# First");
        assert_eq!(chunks[1].line_start, 4);
    }
}
