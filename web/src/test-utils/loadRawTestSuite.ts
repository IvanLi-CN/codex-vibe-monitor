import * as React from "react";
import ts from "typescript";

export function loadRawTestSuite(source: string, scope: Record<string, unknown>): void {
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: {
      jsx: ts.JsxEmit.React,
      module: ts.ModuleKind.None,
      target: ts.ScriptTarget.ES2020,
    },
    fileName: "raw-test-suite.tsx",
  });
  const code = outputText.replace(/^"use strict";\s*/, "");
  new Function("scope", "React", `with (scope) { ${code} }`)(scope, React);
}
