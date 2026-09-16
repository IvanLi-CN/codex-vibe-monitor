use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::model::{Diagnostic, FunctionMetric, ParseError, SourceFile};
use crate::parsers::Analysis;

const STRUCTURAL_LINTS: [(&str, &str); 3] = [
    ("too_many_lines", "clippy::too_many_lines"),
    ("too_many_arguments", "clippy::too_many_arguments"),
    ("excessive_nesting", "clippy::excessive_nesting"),
];

struct FunctionVisitor {
    functions: Vec<FunctionMetric>,
}

impl FunctionVisitor {
    fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }
}

impl<'ast> Visit<'ast> for FunctionVisitor {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.functions.push(function_metric(
            item.sig.ident.to_string(),
            item.sig.span().start().line,
            item.span().end().line,
            item.sig
                .inputs
                .iter()
                .filter(|input| !matches!(input, syn::FnArg::Receiver(_)))
                .count(),
            max_nesting(&item.block),
        ));
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.functions.push(function_metric(
            item.sig.ident.to_string(),
            item.sig.span().start().line,
            item.span().end().line,
            item.sig
                .inputs
                .iter()
                .filter(|input| !matches!(input, syn::FnArg::Receiver(_)))
                .count(),
            max_nesting(&item.block),
        ));
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.functions.push(function_metric(
            item.sig.ident.to_string(),
            item.sig.span().start().line,
            item.span().end().line,
            item.sig
                .inputs
                .iter()
                .filter(|input| !matches!(input, syn::FnArg::Receiver(_)))
                .count(),
            item.default.as_ref().map(max_nesting).unwrap_or(0),
        ));
        syn::visit::visit_trait_item_fn(self, item);
    }
}

fn function_metric(
    name: String,
    line: usize,
    end_line: usize,
    parameters: usize,
    nesting: usize,
) -> FunctionMetric {
    FunctionMetric {
        name,
        line,
        lines: end_line.saturating_sub(line) as u64 + 1,
        parameters: parameters as u64,
        nesting: nesting as u64,
    }
}

pub fn analyze(file: &SourceFile) -> Result<Analysis, ParseError> {
    let syntax = syn::parse_file(&file.content).map_err(|error| ParseError {
        path: file.path.clone(),
        message: format!("syn parse error: {error}"),
    })?;
    let mut visitor = FunctionVisitor::new();
    visitor.visit_file(&syntax);
    let mut test_visitor = InlineTestVisitor::new();
    test_visitor.visit_file(&syntax);
    let mut suppression_diagnostics = Vec::new();
    suppression_diagnostics.extend(structural_suppressions(&file.path, &file.content));
    Ok(Analysis {
        functions: visitor.functions,
        inline_tests: test_visitor.count,
        diagnostics: suppression_diagnostics,
    })
}

struct InlineTestVisitor {
    count: u64,
}

impl InlineTestVisitor {
    fn new() -> Self {
        Self { count: 0 }
    }
}

impl<'ast> Visit<'ast> for InlineTestVisitor {
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        let name = attribute
            .path()
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        let cfg_mentions_test = matches!(
            &attribute.meta,
            syn::Meta::List(list) if list.tokens.to_string().contains("test")
        );
        if matches!(name.as_deref(), Some("test"))
            || matches!(name.as_deref(), Some("cfg" | "cfg_attr")) && cfg_mentions_test
        {
            self.count += 1;
        }
        syn::visit::visit_attribute(self, attribute);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.ident == "tests" {
            self.count += 1;
        }
        syn::visit::visit_item_mod(self, item);
    }
}

fn structural_suppressions(path: &str, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (short_name, lint_name) in STRUCTURAL_LINTS {
        let mut search_from = 0;
        while let Some(relative) = source[search_from..].find(short_name) {
            let offset = search_from + relative;
            search_from = offset + short_name.len();
            let Some(attribute_start) = source[..offset].rfind("#[") else {
                continue;
            };
            let Some(relative_end) = source[offset..].find(']') else {
                continue;
            };
            let attribute_end = offset + relative_end + 1;
            let attribute = &source[attribute_start..attribute_end];
            if !attribute.contains("allow")
                && !attribute.contains("expect")
                && !attribute.contains("cfg_attr")
            {
                continue;
            }
            let line = source[..attribute_start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            diagnostics.push(Diagnostic::new(
                "rust-structural-suppression",
                path,
                format!("line:{line}:{lint_name}"),
                1,
                0,
            ));
        }
    }
    diagnostics
}

struct NestingVisitor {
    depth: usize,
    max: usize,
}

impl NestingVisitor {
    fn new() -> Self {
        Self { depth: 0, max: 0 }
    }

    fn enter(&mut self) {
        self.depth += 1;
        self.max = self.max.max(self.depth);
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

impl<'ast> Visit<'ast> for NestingVisitor {
    fn visit_item_fn(&mut self, _item: &'ast syn::ItemFn) {}

    fn visit_impl_item_fn(&mut self, _item: &'ast syn::ImplItemFn) {}

    fn visit_expr_if(&mut self, expression: &'ast syn::ExprIf) {
        self.enter();
        syn::visit::visit_expr_if(self, expression);
        self.leave();
    }

    fn visit_expr_for_loop(&mut self, expression: &'ast syn::ExprForLoop) {
        self.enter();
        syn::visit::visit_expr_for_loop(self, expression);
        self.leave();
    }

    fn visit_expr_while(&mut self, expression: &'ast syn::ExprWhile) {
        self.enter();
        syn::visit::visit_expr_while(self, expression);
        self.leave();
    }

    fn visit_expr_loop(&mut self, expression: &'ast syn::ExprLoop) {
        self.enter();
        syn::visit::visit_expr_loop(self, expression);
        self.leave();
    }

    fn visit_expr_match(&mut self, expression: &'ast syn::ExprMatch) {
        self.enter();
        syn::visit::visit_expr_match(self, expression);
        self.leave();
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        self.enter();
        syn::visit::visit_expr_try(self, expression);
        self.leave();
    }

    fn visit_expr_try_block(&mut self, expression: &'ast syn::ExprTryBlock) {
        self.enter();
        syn::visit::visit_expr_try_block(self, expression);
        self.leave();
    }
}

fn max_nesting(block: &syn::Block) -> usize {
    let mut visitor = NestingVisitor::new();
    visitor.visit_block(block);
    visitor.max
}
