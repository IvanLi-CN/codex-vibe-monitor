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

const tintedEndpointBadgeRule =
  /\[data-theme="vibe-dark"\]\s+\.invocation-endpoint-badge\[data-endpoint-kind="(?<kind>[^"]+)"\]\s*\{(?<declarations>[\s\S]*?)^\s*\}/gm;
const imageEditEndpointBadgeRule =
  /(?:\[data-theme="vibe-dark"\]\s+)?\.invocation-endpoint-badge\[data-endpoint-kind="image_edit"\]\s*\{(?<declarations>[\s\S]*?)^\s*\}/gm;
const imageEditInkToken =
  /--endpoint-ink-image-edit:\s*oklch\((?<lightness>\d+(?:\.\d+)?)%\s+(?<chroma>\d+(?:\.\d+)?)\s+(?<hue>\d+(?:\.\d+)?)\)/g;

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

  it("uses tone ink instead of filled-content ink on tinted endpoint badges", () => {
    const css = readFileSync(join(sourceRoot, "index.css"), "utf8");
    const offenders = Array.from(css.matchAll(tintedEndpointBadgeRule)).flatMap((match) => {
      const declarations = match.groups?.declarations ?? "";
      const kind = match.groups?.kind ?? "unknown";
      const hasTintedSurface = /background-color:\s*color-mix\(/.test(declarations);
      const usesFilledContentInk =
        /color:\s*[^;]*--color-(?:primary|accent|info|success|warning|error)-content/.test(
          declarations,
        );

      return hasTintedSurface && usesFilledContentInk ? [kind] : [];
    });

    expect(offenders).toEqual([]);
  });

  it("keeps image edit endpoint ink amber and away from neutral black or white", () => {
    const css = readFileSync(join(sourceRoot, "index.css"), "utf8");
    const declarations = Array.from(css.matchAll(imageEditEndpointBadgeRule)).map(
      (match) => match.groups?.declarations ?? "",
    );
    const tokens = Array.from(css.matchAll(imageEditInkToken)).map((match) => ({
      lightness: Number.parseFloat(match.groups?.lightness ?? "NaN"),
      chroma: Number.parseFloat(match.groups?.chroma ?? "NaN"),
      hue: Number.parseFloat(match.groups?.hue ?? "NaN"),
    }));

    expect(declarations).toHaveLength(2);
    for (const declaration of declarations) {
      expect(declaration).toMatch(/color:\s*var\(--endpoint-ink-image-edit\)/);
      expect(declaration).not.toMatch(/--(?:tone-ink-accent|color-(?:base|neutral)-content)/);
    }
    expect(tokens).toHaveLength(2);
    for (const token of tokens) {
      expect(token.lightness).toBeGreaterThanOrEqual(40);
      expect(token.lightness).toBeLessThanOrEqual(80);
      expect(token.chroma).toBeGreaterThanOrEqual(0.1);
      expect(token.hue).toBeGreaterThanOrEqual(55);
      expect(token.hue).toBeLessThanOrEqual(95);
    }
  });
});
