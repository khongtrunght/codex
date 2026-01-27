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
enum SyntaxCategory {
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
    Type,
    TypeBuiltin,
    Variable,
    VariableBuiltin,
    VariableParameter,
}

impl SyntaxCategory {
    /// All categories in order - must match HIGHLIGHT_NAMES order.
    const ALL: [Self; 29] = [
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
        Self::Type,
        Self::TypeBuiltin,
        Self::Variable,
        Self::VariableBuiltin,
        Self::VariableParameter,
    ];

    fn style(self) -> Style {
        // Tokyo Night official color palette
        let red = Color::Rgb(247, 118, 142); // #f7768e
        let orange = Color::Rgb(255, 158, 100); // #ff9e64
        let yellow = Color::Rgb(224, 175, 104); // #e0af68
        let green = Color::Rgb(158, 206, 106); // #9ece6a
        let teal_green = Color::Rgb(115, 218, 202); // #73daca
        let light_teal = Color::Rgb(180, 249, 248); // #b4f9f8
        let teal = Color::Rgb(42, 195, 222); // #2ac3de
        let cyan = Color::Rgb(125, 207, 255); // #7dcfff
        let blue = Color::Rgb(122, 162, 247); // #7aa2f7
        let purple = Color::Rgb(187, 154, 247); // #bb9af7
        let foreground = Color::Rgb(192, 202, 245); // #c0caf5
        let comment = Color::Rgb(86, 95, 137); // #565f89

        match self {
            Self::Comment => Style::default().fg(comment).italic().dim(),
            Self::Keyword => Style::default().fg(purple).bold(),
            Self::Function | Self::FunctionMacro => Style::default().fg(blue),
            Self::FunctionBuiltin => Style::default().fg(teal),
            Self::Constructor => Style::default().fg(cyan),
            Self::String => Style::default().fg(green),
            Self::StringEscape | Self::StringSpecial => Style::default().fg(light_teal),
            Self::Escape => Style::default().fg(light_teal),
            Self::Number => Style::default().fg(orange),
            Self::Constant | Self::ConstantBuiltin => Style::default().fg(orange).bold(),
            Self::Type | Self::TypeBuiltin => Style::default().fg(cyan),
            Self::Tag => Style::default().fg(red),
            Self::Operator => Style::default().fg(purple).dim(),
            Self::Punctuation
            | Self::PunctuationBracket
            | Self::PunctuationDelimiter
            | Self::PunctuationSpecial => Style::default().fg(foreground).dim(),
            Self::Variable => Style::default().fg(foreground),
            Self::VariableBuiltin => Style::default().fg(red),
            Self::VariableParameter => Style::default().fg(yellow),
            Self::Property => Style::default().fg(teal_green),
            Self::Attribute => Style::default().fg(orange),
            Self::Label => Style::default().fg(teal_green),
            Self::Namespace => Style::default().fg(cyan),
            Self::Embedded => Style::default(),
        }
    }
}

/// Highlight names that tree-sitter queries produce.
/// Order must match SyntaxCategory::ALL (expanded from color-diff).
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
pub fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
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
    let iterator = match highlighter.highlight(config, code.as_bytes(), None, |_| None) {
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
                let style = highlight_stack.last().map(|h| category_for(*h).style());
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
pub fn is_language_supported(lang: &str) -> bool {
    config_for_language(lang).is_some()
}

#[cfg(test)]
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
        let lines = highlight_code_to_lines(code, "unknown_lang");
        assert_eq!(reconstructed(&lines), code);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn empty_code_returns_empty_line() {
        let lines = highlight_code_to_lines("", "rust");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.is_empty());
    }

    #[test]
    fn rust_code_preserves_content() {
        let code = "fn main() {\n    println!(\"Hello\");\n}";
        let lines = highlight_code_to_lines(code, "rust");
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn python_code_preserves_content() {
        let code = "def hello():\n    print(\"world\")";
        let lines = highlight_code_to_lines(code, "python");
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn javascript_code_preserves_content() {
        let code = "const x = 42;\nconsole.log(x);";
        let lines = highlight_code_to_lines(code, "javascript");
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn typescript_code_preserves_content() {
        let code = "const x: number = 42;";
        let lines = highlight_code_to_lines(code, "typescript");
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn go_code_preserves_content() {
        let code = "func main() {\n    fmt.Println(\"hi\")\n}";
        let lines = highlight_code_to_lines(code, "go");
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn json_code_preserves_content() {
        let code = "{\"key\": \"value\", \"num\": 42}";
        let lines = highlight_code_to_lines(code, "json");
        assert_eq!(reconstructed(&lines), code);
    }

    #[test]
    fn bash_code_preserves_content() {
        let code = "echo \"hello world\"";
        let lines = highlight_code_to_lines(code, "bash");
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
    }

    #[test]
    fn rust_highlights_keywords() {
        let code = "fn main() {}";
        let lines = highlight_code_to_lines(code, "rust");
        // Find the 'fn' span and verify it has the Tokyo Night keyword color.
        let fn_span = lines[0].spans.iter().find(|s| s.content.as_ref() == "fn");
        assert!(fn_span.is_some(), "should find 'fn' span");
        let style = fn_span.map(|s| s.style);
        assert!(
            style.is_some_and(|s| s.fg == Some(Color::Rgb(187, 154, 247))),
            "fn should use the keyword color"
        );
    }

    #[test]
    fn python_highlights_strings() {
        let code = "x = \"hello\"";
        let lines = highlight_code_to_lines(code, "python");
        // Strings should use the Tokyo Night string color.
        let string_span = lines[0].spans.iter().find(|s| s.content.contains("hello"));
        assert!(string_span.is_some(), "should find string span");
        let style = string_span.map(|s| s.style);
        assert!(
            style.is_some_and(|s| s.fg == Some(Color::Rgb(158, 206, 106))),
            "string should use the string color"
        );
    }
}
