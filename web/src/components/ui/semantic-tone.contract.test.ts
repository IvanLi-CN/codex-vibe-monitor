import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));
const sourceRoot = join(projectRoot, "src");
const lowOpacityRange = "(?:[1-9]|[1-6]\\d|7\\d)";
const lowOpacityFilledContentPatterns = [
  new RegExp(
    `bg-(primary|accent|info|success|warning|error)/${lowOpacityRange}(?!\\d)[^"'\\\`\\n]*text-\\1-content`,
    "g",
  ),
  new RegExp(
    `text-(primary|accent|info|success|warning|error)-content[^"'\\\`\\n]*bg-\\1/${lowOpacityRange}(?!\\d)`,
    "g",
  ),
];

function walkSourceFiles(root: string): string[] {
  return readdirSync(root).flatMap((entry) => {
    const nextPath = join(root, entry);
    const stats = statSync(nextPath);
    if (stats.isDirectory()) {
      return walkSourceFiles(nextPath);
    }
    if (!/\.(css|ts|tsx)$/.test(nextPath)) {
      return [];
    }
    return [nextPath];
  });
}

describe("semantic tone source contract", () => {
  it("blocks filled-content text tokens on low-opacity semantic surfaces", () => {
    const offenders = walkSourceFiles(sourceRoot)
      .map((filePath) => ({
        relativePath: relative(sourceRoot, filePath).replaceAll("\\", "/"),
        content: readFileSync(filePath, "utf8"),
      }))
      .flatMap(({ relativePath, content }) =>
        lowOpacityFilledContentPatterns.flatMap((pattern) =>
          Array.from(content.matchAll(pattern)).map((match) => ({
            relativePath,
            snippet: match[0],
          })),
        ),
      );

    expect(offenders).toEqual([]);
  });
});
