//! Multi-language syntax highlighting for code blocks.
//!
//! Provides syntax highlighting for common programming languages using tree-sitter.
//! Falls back to plain text for unsupported languages.

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use std::sync::OnceLock;
use tree_sitter_highlight::Highlight;
use tree_sitter_highlight::HighlightConfiguration;
use tree_sitter_highlight::HighlightEvent;
use tree_sitter_highlight::Highlighter;

/// Unified highlight categories that work across all languages.
/// These map to tree-sitter highlight capture names (expanded from color-diff).
#[derive(Copy, Clone, Debug)]
pub(crate) enum SyntaxCategory {
    Attribute,
    Comment,
    Constant,
    ConstantBuiltin,
    Constructor,
    Embedded,
    Escape,
    Function,
    FunctionBuiltin,
    FunctionMacro,
    Keyword,
    Label,
    Namespace,
    Number,
    Operator,
    Property,
    Punctuation,
    PunctuationBracket,
    PunctuationDelimiter,
    PunctuationSpecial,
    String,
    StringEscape,
    StringSpecial,
    Tag,
    TextEmphasis,
    TextLiteral,
    TextReference,
    TextStrong,
    TextTitle,
    TextUri,
    Type,
    TypeBuiltin,
    Variable,
    VariableBuiltin,
    VariableParameter,
}

pub trait Theme {
    fn style(&self, category: SyntaxCategory) -> Style;
}

pub struct DefaultTheme;
pub struct MarkdownTheme;

impl Theme for DefaultTheme {
    fn style(&self, category: SyntaxCategory) -> Style {
        let green = Color::Indexed(148); // built_in, type, property
        let yellow = Color::Yellow; // function, attribute
        let magenta = Color::Indexed(141); // keyword, number, label
        let blue = Color::Blue; // namespace, constant
        let red = Color::Red; // variable.builtin
        let grey = Color::DarkGray; // comment, tag
        let white = Color::White; // punctuation

        match category {
            SyntaxCategory::Comment => Style::default().fg(grey).italic(),
            SyntaxCategory::Keyword | SyntaxCategory::TextTitle => {
                Style::default().fg(Color::Indexed(208)).bold()
            }
            SyntaxCategory::Function => Style::default().fg(yellow),
            SyntaxCategory::FunctionMacro => Style::default().fg(green),
            SyntaxCategory::FunctionBuiltin => Style::default().fg(green),
            SyntaxCategory::Constructor => Style::default().fg(green).bold(),
            SyntaxCategory::String
            | SyntaxCategory::TextLiteral
            | SyntaxCategory::TextReference => Style::default().fg(Color::Indexed(203)),
            SyntaxCategory::TextUri => Style::default().fg(Color::Indexed(203)).underlined(),
            SyntaxCategory::StringEscape | SyntaxCategory::StringSpecial => {
                Style::default().fg(magenta)
            }
            SyntaxCategory::Escape => Style::default().fg(magenta),
            SyntaxCategory::Number => Style::default().fg(magenta),
            SyntaxCategory::Constant | SyntaxCategory::ConstantBuiltin => {
                Style::default().fg(blue).bold()
            }
            SyntaxCategory::Type | SyntaxCategory::TypeBuiltin => {
                Style::default().fg(Color::Indexed(197))
            }
            SyntaxCategory::Tag => Style::default().fg(grey),
            SyntaxCategory::Attribute => Style::default().fg(yellow),
            SyntaxCategory::TextEmphasis => Style::default().italic(),
            SyntaxCategory::TextStrong => Style::default().bold(),

            SyntaxCategory::Operator => Style::default().fg(magenta).dim(),
            SyntaxCategory::Punctuation => Style::default().fg(white).dim(),
            SyntaxCategory::PunctuationBracket => Style::default().fg(white).dim(),
            SyntaxCategory::PunctuationDelimiter => Style::default().fg(white).dim(),
            SyntaxCategory::PunctuationSpecial => Style::default().fg(magenta).dim(),
            SyntaxCategory::Variable => Style::default().fg(Color::Rgb(255, 255, 255)),
            SyntaxCategory::VariableBuiltin => Style::default().fg(red).italic(),
            SyntaxCategory::VariableParameter => Style::default().fg(yellow).italic(),
            SyntaxCategory::Property => Style::default().fg(green),
            SyntaxCategory::Label => Style::default().fg(magenta),
            SyntaxCategory::Namespace => Style::default().fg(blue).dim(),
            SyntaxCategory::Embedded => Style::default().fg(yellow).dim(),
        }
    }
}

impl Theme for MarkdownTheme {
    fn style(&self, category: SyntaxCategory) -> Style {
        let blue = Color::Blue; // keyword, literal, class
        let cyan = Color::Cyan; // built_in, type, attr
        let green = Color::Green; // number, comment
        let red = Color::Red; // string, regexp
        let yellow = Color::Yellow; // function
        let magenta = Color::Magenta; // label, namespace
        let grey = Color::DarkGray; // meta, tag
        let white = Color::White; // punctuation

        match category {
            SyntaxCategory::Comment => Style::default().fg(green).italic(),
            SyntaxCategory::Keyword | SyntaxCategory::TextTitle => Style::default().fg(blue).bold(),
            SyntaxCategory::Function => Style::default().fg(yellow),
            SyntaxCategory::FunctionMacro => Style::default().fg(cyan),
            SyntaxCategory::FunctionBuiltin => Style::default().fg(cyan),
            SyntaxCategory::Constructor => Style::default().fg(cyan).bold(),
            SyntaxCategory::String
            | SyntaxCategory::TextLiteral
            | SyntaxCategory::TextReference => Style::default().fg(red),
            SyntaxCategory::TextUri => Style::default().fg(red).underlined(),
            SyntaxCategory::StringEscape | SyntaxCategory::StringSpecial => {
                Style::default().fg(magenta)
            }
            SyntaxCategory::Escape => Style::default().fg(magenta),
            SyntaxCategory::Number => Style::default().fg(green),
            SyntaxCategory::Constant | SyntaxCategory::ConstantBuiltin => {
                Style::default().fg(blue).bold()
            }
            SyntaxCategory::Type | SyntaxCategory::TypeBuiltin => Style::default().fg(cyan).dim(),
            SyntaxCategory::Tag => Style::default().fg(grey),
            SyntaxCategory::Attribute => Style::default().fg(cyan),
            SyntaxCategory::TextEmphasis => Style::default().italic(),
            SyntaxCategory::TextStrong => Style::default().bold(),

            SyntaxCategory::Operator => Style::default().fg(magenta).dim(),
            SyntaxCategory::Punctuation => Style::default().fg(white).dim(),
            SyntaxCategory::PunctuationBracket => Style::default().fg(white).dim(),
            SyntaxCategory::PunctuationDelimiter => Style::default().fg(white).dim(),
            SyntaxCategory::PunctuationSpecial => Style::default().fg(magenta).dim(),
            SyntaxCategory::Variable => Style::default(),
            SyntaxCategory::VariableBuiltin => Style::default().fg(red).italic(),
            SyntaxCategory::VariableParameter => Style::default().fg(yellow).italic(),
            SyntaxCategory::Property => Style::default().fg(cyan),
            SyntaxCategory::Label => Style::default().fg(magenta),
            SyntaxCategory::Namespace => Style::default().fg(blue).dim(),
            SyntaxCategory::Embedded => Style::default().fg(yellow).dim(),
        }
    }
}

impl SyntaxCategory {
    /// All categories in order - must match HIGHLIGHT_NAMES order.
    const ALL: [Self; 35] = [
        Self::Attribute,
        Self::Comment,
        Self::Constant,
        Self::ConstantBuiltin,
        Self::Constructor,
        Self::Embedded,
        Self::Escape,
        Self::Function,
        Self::FunctionBuiltin,
        Self::FunctionMacro,
        Self::Keyword,
        Self::Label,
        Self::Namespace,
        Self::Number,
        Self::Operator,
        Self::Property,
        Self::Punctuation,
        Self::PunctuationBracket,
        Self::PunctuationDelimiter,
        Self::PunctuationSpecial,
        Self::String,
        Self::StringEscape,
        Self::StringSpecial,
        Self::Tag,
        Self::TextEmphasis,
        Self::TextLiteral,
        Self::TextReference,
        Self::TextStrong,
        Self::TextTitle,
        Self::TextUri,
        Self::Type,
        Self::TypeBuiltin,
        Self::Variable,
        Self::VariableBuiltin,
        Self::VariableParameter,
    ];

    fn style(self, theme: &impl Theme) -> Style {
        theme.style(self)
    }
}

/// Highlight names that tree-sitter queries produce.
/// Order must match SyntaxCategory::ALL.
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "keyword",
    "label",
    "namespace",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.escape",
    "string.special",
    "tag",
    "text.emphasis",
    "text.literal",
    "text.reference",
    "text.strong",
    "text.title",
    "text.uri",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

fn category_for(highlight: Highlight) -> SyntaxCategory {
    SyntaxCategory::ALL
        .get(highlight.0)
        .copied()
        .unwrap_or(SyntaxCategory::Variable)
}

// Per-language configurations stored in OnceLock for lazy initialization.
// We store Option<HighlightConfiguration> to handle initialization failures gracefully.

static RUST_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static PYTHON_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static JAVASCRIPT_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static TYPESCRIPT_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static TSX_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static GO_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static JSON_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static BASH_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static MARKDOWN_BLOCK_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static MARKDOWN_INLINE_CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
static MARKDOWN_BLOCK_HIGHLIGHT_QUERY: OnceLock<String> = OnceLock::new();
static MARKDOWN_BLOCK_INJECTION_QUERY: OnceLock<String> = OnceLock::new();

fn make_rust_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        tree_sitter_rust::INJECTIONS_QUERY,
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn rust_config() -> Option<&'static HighlightConfiguration> {
    RUST_CONFIG.get_or_init(make_rust_config).as_ref()
}

fn make_python_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_python::LANGUAGE.into(),
        "python",
        tree_sitter_python::HIGHLIGHTS_QUERY,
        "",
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn python_config() -> Option<&'static HighlightConfiguration> {
    PYTHON_CONFIG.get_or_init(make_python_config).as_ref()
}

fn make_javascript_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::INJECTIONS_QUERY,
        tree_sitter_javascript::LOCALS_QUERY,
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn javascript_config() -> Option<&'static HighlightConfiguration> {
    JAVASCRIPT_CONFIG
        .get_or_init(make_javascript_config)
        .as_ref()
}

fn make_typescript_config() -> Option<HighlightConfiguration> {
    // TypeScript uses JavaScript highlights + TypeScript-specific ones
    let combined_query = format!(
        "{}\n{}",
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_typescript::HIGHLIGHTS_QUERY
    );
    let mut config = HighlightConfiguration::new(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "typescript",
        &combined_query,
        tree_sitter_javascript::INJECTIONS_QUERY,
        tree_sitter_typescript::LOCALS_QUERY,
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn typescript_config() -> Option<&'static HighlightConfiguration> {
    TYPESCRIPT_CONFIG
        .get_or_init(make_typescript_config)
        .as_ref()
}

fn make_tsx_config() -> Option<HighlightConfiguration> {
    let combined_query = format!(
        "{}\n{}",
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_typescript::HIGHLIGHTS_QUERY
    );
    let mut config = HighlightConfiguration::new(
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        "tsx",
        &combined_query,
        tree_sitter_javascript::INJECTIONS_QUERY,
        tree_sitter_typescript::LOCALS_QUERY,
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn tsx_config() -> Option<&'static HighlightConfiguration> {
    TSX_CONFIG.get_or_init(make_tsx_config).as_ref()
}

fn make_go_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_go::LANGUAGE.into(),
        "go",
        tree_sitter_go::HIGHLIGHTS_QUERY,
        "",
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn go_config() -> Option<&'static HighlightConfiguration> {
    GO_CONFIG.get_or_init(make_go_config).as_ref()
}

fn make_json_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_json::LANGUAGE.into(),
        "json",
        tree_sitter_json::HIGHLIGHTS_QUERY,
        "",
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn json_config() -> Option<&'static HighlightConfiguration> {
    JSON_CONFIG.get_or_init(make_json_config).as_ref()
}

fn make_bash_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        tree_sitter_bash::HIGHLIGHT_QUERY,
        "",
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn bash_config() -> Option<&'static HighlightConfiguration> {
    BASH_CONFIG.get_or_init(make_bash_config).as_ref()
}

fn make_markdown_block_config() -> Option<HighlightConfiguration> {
    let highlight_query = markdown_block_highlight_query();
    let injection_query = markdown_block_injection_query();
    let mut config = HighlightConfiguration::new(
        tree_sitter_md::LANGUAGE.into(),
        "markdown",
        highlight_query,
        injection_query,
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn markdown_block_highlight_query() -> &'static str {
    MARKDOWN_BLOCK_HIGHLIGHT_QUERY
        .get_or_init(|| {
            let mut lines: Vec<&str> = tree_sitter_md::HIGHLIGHT_QUERY_BLOCK
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    trimmed != "(fenced_code_block)" && trimmed != "(code_fence_content) @none"
                })
                .collect();
            if !lines.is_empty() {
                lines.push("");
            }
            let mut query = lines.join("\n");
            if !query.is_empty() && !query.ends_with('\n') {
                query.push('\n');
            }
            query.push_str("(info_string\n  (language) @label)\n");
            query
        })
        .as_str()
}

fn markdown_block_injection_query() -> &'static str {
    MARKDOWN_BLOCK_INJECTION_QUERY
        .get_or_init(|| {
            let mut query = tree_sitter_md::INJECTION_QUERY_BLOCK.to_string();
            query = query.replace(
                "(fenced_code_block\n  (info_string\n    (language) @injection.language)\n  (code_fence_content) @injection.content)\n\n",
                "((fenced_code_block\n  (info_string\n    (language) @injection.language)\n  (code_fence_content) @injection.content)\n (#set! injection.include-children))\n\n",
            );
            query = query.replace(
                "((inline) @injection.content\n  (#set! injection.language \"markdown_inline\"))",
                "((inline) @injection.content\n  (#set! injection.language \"markdown_inline\")\n  (#set! injection.include-children))",
            );
            query
        })
        .as_str()
}

fn markdown_block_config() -> Option<&'static HighlightConfiguration> {
    MARKDOWN_BLOCK_CONFIG
        .get_or_init(make_markdown_block_config)
        .as_ref()
}

fn make_markdown_inline_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_md::INLINE_LANGUAGE.into(),
        "markdown_inline",
        tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
        tree_sitter_md::INJECTION_QUERY_INLINE,
        "",
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn markdown_inline_config() -> Option<&'static HighlightConfiguration> {
    MARKDOWN_INLINE_CONFIG
        .get_or_init(make_markdown_inline_config)
        .as_ref()
}

/// Get the highlight configuration for a language by name.
/// Returns None for unsupported languages.
fn config_for_language(lang: &str) -> Option<&'static HighlightConfiguration> {
    match lang.to_lowercase().as_str() {
        "rust" | "rs" => rust_config(),
        "python" | "py" => python_config(),
        "javascript" | "js" => javascript_config(),
        "typescript" | "ts" => typescript_config(),
        "tsx" => tsx_config(),
        "go" | "golang" => go_config(),
        "json" => json_config(),
        "bash" | "sh" | "shell" | "zsh" => bash_config(),
        "markdown" | "md" | "mdx" => markdown_block_config(),
        "markdown_inline" => markdown_inline_config(),
        _ => None,
    }
}

fn push_segment(lines: &mut Vec<Line<'static>>, segment: &str, style: Option<Style>) {
    for (i, part) in segment.split('\n').enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        if part.is_empty() {
            continue;
        }
        let span = match style {
            Some(style) => Span::styled(part.to_string(), style),
            None => part.to_string().into(),
        };
        if let Some(last) = lines.last_mut() {
            last.spans.push(span);
        }
    }
}

/// Highlight code in the specified language and return styled lines.
///
/// Falls back to plain text (no highlighting) for:
/// - Unsupported languages
/// - Parse errors
/// - Empty input
pub fn highlight_code_to_lines<T: Theme>(code: &str, lang: &str, theme: &T) -> Vec<Line<'static>> {
    if code.is_empty() {
        return vec![Line::from("")];
    }

    // Windows has known stack overflow issues with tree-sitter
    if cfg!(target_os = "windows") {
        return plain_text_lines(code);
    }

    let Some(config) = config_for_language(lang) else {
        return plain_text_lines(code);
    };

    let mut highlighter = Highlighter::new();
    let iterator = match highlighter.highlight(config, code.as_bytes(), None, |lang| {
        config_for_language(lang)
    }) {
        Ok(iter) => iter,
        Err(_) => return plain_text_lines(code),
    };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    let mut highlight_stack: Vec<Highlight> = Vec::new();

    for event in iterator {
        match event {
            Ok(HighlightEvent::HighlightStart(highlight)) => highlight_stack.push(highlight),
            Ok(HighlightEvent::HighlightEnd) => {
                highlight_stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                if start == end {
                    continue;
                }
                let style = highlight_stack
                    .last()
                    .map(|h| category_for(*h).style(theme));
                push_segment(&mut lines, &code[start..end], style);
            }
            Err(_) => return plain_text_lines(code),
        }
    }

    if lines.is_empty() {
        vec![Line::from("")]
    } else {
        lines
    }
}

/// Convert code to plain text lines (no highlighting).
fn plain_text_lines(code: &str) -> Vec<Line<'static>> {
    code.lines().map(|l| Line::from(l.to_string())).collect()
}

/// Check if a language is supported for syntax highlighting.
#[cfg(test)]
fn is_language_supported(lang: &str) -> bool {
    config_for_language(lang).is_some()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn reconstructed(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|sp| sp.content.clone())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn unsupported_language_returns_plain_text() {
        let code = "some random code";
        let lines = highlight_code_to_lines(code, "unknown_lang", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn empty_code_returns_empty_line() {
        let lines = highlight_code_to_lines("", "rust", &DefaultTheme);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.is_empty());
    }

    #[test]
    fn rust_code_preserves_content() {
        let code = "fn main() {\n    println!(\"Hello\");\n}";
        let lines = highlight_code_to_lines(code, "rust", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn python_code_preserves_content() {
        let code = "def hello():\n    print(\"world\")";
        let lines = highlight_code_to_lines(code, "python", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn javascript_code_preserves_content() {
        let code = "const x = 42;\nconsole.log(x);";
        let lines = highlight_code_to_lines(code, "javascript", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn typescript_code_preserves_content() {
        let code = "const x: number = 42;";
        let lines = highlight_code_to_lines(code, "typescript", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn go_code_preserves_content() {
        let code = "func main() {\n    fmt.Println(\"hi\")\n}";
        let lines = highlight_code_to_lines(code, "go", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn json_code_preserves_content() {
        let code = "{\"key\": \"value\", \"num\": 42}";
        let lines = highlight_code_to_lines(code, "json", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn bash_code_preserves_content() {
        let code = "echo \"hello world\"";
        let lines = highlight_code_to_lines(code, "bash", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn markdown_code_preserves_content() {
        let code = "# Hello\n\n- item";
        let lines = highlight_code_to_lines(code, "markdown", &DefaultTheme);
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn language_aliases_work() {
        assert!(is_language_supported("rs"));
        assert!(is_language_supported("py"));
        assert!(is_language_supported("js"));
        assert!(is_language_supported("ts"));
        assert!(is_language_supported("sh"));
        assert!(is_language_supported("golang"));
        assert!(is_language_supported("md"));
    }
}
