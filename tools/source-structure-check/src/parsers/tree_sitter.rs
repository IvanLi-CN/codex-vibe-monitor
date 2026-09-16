use tree_sitter::{Node, Parser};

use crate::model::{FunctionMetric, ParseError, SourceFile};
use crate::parsers::Analysis;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LanguageKind {
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Bash,
}

pub fn analyze(file: &SourceFile, extension: &str) -> Result<Analysis, ParseError> {
    let language = match extension {
        "ts" => LanguageKind::TypeScript,
        "tsx" => LanguageKind::Tsx,
        "js" | "jsx" => LanguageKind::JavaScript,
        "py" => LanguageKind::Python,
        "sh" => LanguageKind::Bash,
        _ => {
            return Ok(Analysis {
                functions: Vec::new(),
                inline_tests: 0,
                diagnostics: Vec::new(),
            });
        }
    };
    let mut parser = Parser::new();
    let tree_sitter_language = match language {
        LanguageKind::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        LanguageKind::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        LanguageKind::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        LanguageKind::Python => tree_sitter_python::LANGUAGE.into(),
        LanguageKind::Bash => tree_sitter_bash::LANGUAGE.into(),
    };
    parser
        .set_language(&tree_sitter_language)
        .map_err(|error| ParseError {
            path: file.path.clone(),
            message: format!("cannot load {extension} Tree-sitter grammar: {error}"),
        })?;
    let tree = parser
        .parse(file.content.as_bytes(), None)
        .ok_or_else(|| ParseError {
            path: file.path.clone(),
            message: "Tree-sitter returned no syntax tree".to_owned(),
        })?;
    let root = tree.root_node();
    if root.has_error() || contains_missing(root) {
        if matches!(
            language,
            LanguageKind::JavaScript | LanguageKind::TypeScript | LanguageKind::Tsx
        ) {
            // Biome is the repository parser for the frontend surface. Older
            // Tree-sitter TypeScript grammars reject a few valid TS constructs;
            // the Biome pass below remains the fail-closed syntax authority.
            return Ok(Analysis {
                functions: Vec::new(),
                inline_tests: 0,
                diagnostics: Vec::new(),
            });
        }
        return Err(ParseError {
            path: file.path.clone(),
            message: format!(
                "Tree-sitter syntax error near line {} ({})",
                first_error_line(root).map(|error| error.0).unwrap_or(1),
                first_error_line(root)
                    .map(|error| error.1)
                    .unwrap_or_else(|| "unknown node".to_owned())
            ),
        });
    }

    let mut functions = Vec::new();
    collect_functions(root, &file.content, language, &mut functions);
    let inline_tests = count_inline_tests(root, &file.content, language);
    Ok(Analysis {
        functions,
        inline_tests,
        diagnostics: Vec::new(),
    })
}

fn collect_functions(
    node: Node<'_>,
    source: &str,
    language: LanguageKind,
    output: &mut Vec<FunctionMetric>,
) {
    if is_function(node, language) {
        let start = node.start_position().row + 1;
        let end = node.end_position().row + 1;
        output.push(FunctionMetric {
            name: function_name(node, source),
            line: start,
            lines: end.saturating_sub(start) as u64 + 1,
            parameters: parameter_count(node, language),
            nesting: max_nesting(node, language),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, source, language, output);
    }
}

fn is_function(node: Node<'_>, language: LanguageKind) -> bool {
    match language {
        LanguageKind::JavaScript | LanguageKind::TypeScript | LanguageKind::Tsx => matches!(
            node.kind(),
            "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "generator_function"
                | "arrow_function"
                | "method_definition"
        ),
        LanguageKind::Python => node.kind() == "function_definition",
        LanguageKind::Bash => node.kind() == "function_definition",
    }
}

fn function_name(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .and_then(|name| name.utf8_text(source.as_bytes()).ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("anonymous@{}", node.start_position().row + 1))
}

fn parameter_count(node: Node<'_>, language: LanguageKind) -> u64 {
    if language == LanguageKind::Bash {
        return 0;
    }
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return 0;
    };
    let mut count = 0;
    let mut cursor = parameters.walk();
    for child in parameters.named_children(&mut cursor) {
        if is_parameter_node(child.kind(), language) {
            count += 1;
        }
    }
    count
}

fn is_parameter_node(kind: &str, language: LanguageKind) -> bool {
    match language {
        LanguageKind::JavaScript | LanguageKind::TypeScript | LanguageKind::Tsx => matches!(
            kind,
            "identifier"
                | "required_parameter"
                | "optional_parameter"
                | "rest_pattern"
                | "assignment_pattern"
                | "object_pattern"
                | "array_pattern"
        ),
        LanguageKind::Python => matches!(
            kind,
            "identifier"
                | "typed_parameter"
                | "default_parameter"
                | "list_splat_pattern"
                | "dictionary_splat_pattern"
        ),
        LanguageKind::Bash => false,
    }
}

fn max_nesting(function: Node<'_>, language: LanguageKind) -> u64 {
    let mut maximum = 0;
    visit_nesting(function, function.id(), language, 0, &mut maximum);
    maximum
}

fn visit_nesting(
    node: Node<'_>,
    root_id: usize,
    language: LanguageKind,
    depth: u64,
    maximum: &mut u64,
) {
    if node.id() != root_id && is_function(node, language) {
        return;
    }
    let next_depth = if node.id() != root_id && is_control(node.kind(), language) {
        depth + 1
    } else {
        depth
    };
    *maximum = (*maximum).max(next_depth);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_nesting(child, root_id, language, next_depth, maximum);
    }
}

fn is_control(kind: &str, language: LanguageKind) -> bool {
    match language {
        LanguageKind::JavaScript | LanguageKind::TypeScript | LanguageKind::Tsx => matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "do_statement"
                | "switch_statement"
                | "try_statement"
                | "with_statement"
        ),
        LanguageKind::Python => matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "while_statement"
                | "try_statement"
                | "match_statement"
        ),
        LanguageKind::Bash => matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "c_style_for_statement"
                | "while_statement"
                | "case_statement"
        ),
    }
}

fn count_inline_tests(node: Node<'_>, source: &str, language: LanguageKind) -> u64 {
    let mut count = 0;
    if matches!(
        language,
        LanguageKind::JavaScript | LanguageKind::TypeScript | LanguageKind::Tsx
    ) && node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .and_then(|function| function.utf8_text(source.as_bytes()).ok())
            .is_some_and(|name| matches!(name, "test" | "it" | "describe" | "suite" | "bench"))
    {
        count += 1;
    }
    if language == LanguageKind::Python
        && node.kind() == "function_definition"
        && node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            .is_some_and(|name| name.starts_with("test"))
    {
        count += 1;
    }
    if language == LanguageKind::Bash
        && node.kind() == "function_definition"
        && node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            .is_some_and(|name| name.starts_with("test_"))
    {
        count += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_inline_tests(child, source, language);
    }
    count
}

fn first_error_line(node: Node<'_>) -> Option<(usize, String)> {
    if node.is_error() || node.is_missing() {
        return Some((node.start_position().row + 1, node.kind().to_owned()));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(error) = first_error_line(child) {
            return Some(error);
        }
    }
    None
}

fn contains_missing(node: Node<'_>) -> bool {
    if node.is_missing() {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).any(contains_missing)
}
