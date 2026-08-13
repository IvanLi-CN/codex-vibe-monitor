import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));
const sourceRoot = join(projectRoot, "src");
const tones = [
  "neutral",
  "primary",
  "secondary",
  "accent",
  "info",
  "success",
  "warning",
  "error",
  "sky",
  "cyan",
  "blue",
  "indigo",
  "violet",
  "fuchsia",
  "teal",
  "emerald",
  "amber",
  "orange",
] as const;
const lowOpacityRange = "(?:[1-9]|[1-6]\\d|7\\d)";
const lowOpacityFilledContentPatterns = [
  new RegExp(
    `bg-(primary|accent|info|success|warning|error)/${lowOpacityRange}(?!\\d)[^"'\\x60\\n]*text-\\1-content`,
    "g",
  ),
  new RegExp(
    `text-(primary|accent|info|success|warning|error)-content[^"'\\x60\\n]*bg-\\1/${lowOpacityRange}(?!\\d)`,
    "g",
  ),
];

function walkSourceFiles(root: string): string[] {
  return readdirSync(root).flatMap((entry) => {
    const nextPath = join(root, entry);
    const stats = statSync(nextPath);
    if (stats.isDirectory()) return walkSourceFiles(nextPath);
    return /\.(css|ts|tsx)$/.test(nextPath) ? [nextPath] : [];
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

  it("keeps the Chip entrypoint unique and free of color overrides", () => {
    const sourceFiles = walkSourceFiles(sourceRoot).map((filePath) => ({
      relativePath: relative(sourceRoot, filePath).replaceAll("\\", "/"),
      content: readFileSync(filePath, "utf8"),
    }));
    const badgeEntrypointImports = sourceFiles.flatMap(({ relativePath, content }) =>
      /(?:from\s+["'][^"']*(?:components\/ui\/badge|\/badge(?:["']|\/))|import\s+["'][^"']*(?:components\/ui\/badge|\/badge(?:["']|\/))|\bBadge(?:Props|Variant)?\b)/.test(
        content,
      )
        ? [relativePath]
        : [],
    );
    expect(badgeEntrypointImports).toEqual([]);

    const chipColorOverrides = sourceFiles.flatMap(({ relativePath, content }) => {
      const offenders: string[] = [];
      for (const match of content.matchAll(/<Chip\b[^>]*>/gs)) {
        const openingTag = match[0] ?? "";
        const className =
          openingTag.match(/className=(?:["']([^"']*)["']|\{([\s\S]*?)\})/)?.[0] ?? "";
        if (
          /(?:^|\s)(?:bg|border|text)-(?:base|primary|secondary|accent|info|success|warning|error|neutral|transparent|teal|sky|cyan|blue|indigo|violet|fuchsia|emerald|amber|orange)(?:\/|\b)/.test(
            className,
          )
        ) {
          offenders.push(`${relativePath}: ${className}`);
        }
      }
      return offenders;
    });
    expect(chipColorOverrides).toEqual([]);
  });

  it("defines opaque surface, border, and ink tokens for every tone in both themes", () => {
    const css = readFileSync(join(sourceRoot, "index.css"), "utf8");
    expect(css).not.toContain("--endpoint-ink-image-edit");
    const declarations = new Map<
      string,
      Array<{ lightness: number; chroma: number; hue: number }>
    >();
    const pattern =
      /--chip-(?<tone>[a-z]+)-(?<part>surface|border|ink):\s*oklch\((?<lightness>\d+(?:\.\d+)?)%\s+(?<chroma>\d+(?:\.\d+)?)\s+(?<hue>\d+(?:\.\d+)?)\)/g;
    for (const match of css.matchAll(pattern)) {
      const tone = match.groups?.tone;
      const part = match.groups?.part;
      if (!tone || !part) continue;
      const key = `${tone}-${part}`;
      const values = declarations.get(key) ?? [];
      values.push({
        lightness: Number(match.groups?.lightness),
        chroma: Number(match.groups?.chroma),
        hue: Number(match.groups?.hue),
      });
      declarations.set(key, values);
    }

    for (const tone of tones) {
      for (const part of ["surface", "border", "ink"] as const) {
        const values = declarations.get(`${tone}-${part}`) ?? [];
        expect(values, `${tone}-${part}`).toHaveLength(2);
        for (const value of values) {
          expect(value.lightness).toBeGreaterThan(5);
          expect(value.lightness).toBeLessThan(99);
          if (part === "ink" && tone !== "neutral" && tone !== "secondary") {
            expect(value.chroma, `${tone} ink chroma`).toBeGreaterThanOrEqual(0.08);
          }
        }
      }
    }
  });

  it("keeps shared tone-ink utility variables defined for non-Chip callers", () => {
    const css = readFileSync(join(sourceRoot, "index.css"), "utf8");
    for (const tone of ["primary", "secondary", "accent", "info", "success", "warning", "error"]) {
      expect(css).toMatch(new RegExp(`--tone-ink-${tone}:`));
    }
  });
});
