use std::collections::BTreeMap;

pub mod biome;
pub mod rust;
pub mod tree_sitter;

use crate::config::FileClass;
use crate::model::{Config, Diagnostic, FunctionMetric, ParseError, SourceFile};

pub struct Analysis {
    pub functions: Vec<FunctionMetric>,
    pub inline_tests: u64,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn analyze(
    file: &SourceFile,
    config: &Config,
    class: FileClass,
) -> Result<Analysis, ParseError> {
    let extension = file
        .path
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or_default();
    let result = match extension {
        "rs" => rust::analyze(file),
        "ts" | "tsx" | "js" | "jsx" => tree_sitter::analyze(file, extension),
        "py" | "sh" => tree_sitter::analyze(file, extension),
        "css" => Ok(Analysis {
            functions: Vec::new(),
            inline_tests: 0,
            diagnostics: Vec::new(),
        }),
        _ => Ok(Analysis {
            functions: Vec::new(),
            inline_tests: 0,
            diagnostics: Vec::new(),
        }),
    }?;

    let mut diagnostics = result.diagnostics;
    let mut functions = result.functions;
    let mut names = BTreeMap::<String, usize>::new();
    for function in &mut functions {
        let count = names.entry(function.name.clone()).or_default();
        *count += 1;
        if *count > 1 {
            function.name = format!("{}#{}", function.name, count);
        }
    }
    for function in &functions {
        if function.lines > config.limits.function_lines {
            diagnostics.push(Diagnostic::new(
                "function-lines",
                &file.path,
                function.subject(),
                function.lines,
                config.limits.function_lines,
            ));
        }
        if function.parameters > config.limits.parameters {
            diagnostics.push(Diagnostic::new(
                "function-parameters",
                &file.path,
                function.subject(),
                function.parameters,
                config.limits.parameters,
            ));
        }
        if function.nesting > config.limits.nesting {
            diagnostics.push(Diagnostic::new(
                "function-nesting",
                &file.path,
                function.subject(),
                function.nesting,
                config.limits.nesting,
            ));
        }
    }
    if class == FileClass::Entry && result.inline_tests > 0 {
        diagnostics.push(Diagnostic::new(
            "entry-inline-tests",
            &file.path,
            "entry",
            result.inline_tests,
            0,
        ));
    }
    Ok(Analysis {
        functions,
        inline_tests: result.inline_tests,
        diagnostics,
    })
}

trait FunctionSubject {
    fn subject(&self) -> String;
}

impl FunctionSubject for FunctionMetric {
    fn subject(&self) -> String {
        if self.name.is_empty() {
            format!("anonymous@{}", self.line)
        } else {
            self.name.clone()
        }
    }
}
