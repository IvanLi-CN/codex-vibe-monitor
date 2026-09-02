import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";

const PORT = Number(process.env.PWA_E2E_PORT ?? "61084");
const distDir = path.resolve(process.cwd(), "dist");
const tempRoot = await mkdtemp(path.join(tmpdir(), "cvm-pwa-test-"));
const variantsRoot = path.join(tempRoot, "variants");
const v1Dir = path.join(variantsRoot, "v1");
const v2Dir = path.join(variantsRoot, "v2");

const installIconPattern =
  /^(favicon|icon-192|icon-512|maskable-192|maskable-512)-[a-f0-9]{12}(\.(?:png|svg))$/;

await mkdir(variantsRoot, { recursive: true });
await cp(distDir, v1Dir, { recursive: true });
await cp(distDir, v2Dir, { recursive: true });

const versionJson = JSON.parse(await readFile(path.join(v1Dir, "version.json"), "utf8"));
const nextVersion = `${String(versionJson.version ?? "0.0.0")}-pwa.1`;
await writeFile(
  path.join(v2Dir, "version.json"),
  `${JSON.stringify({ version: nextVersion }, null, 2)}\n`,
  "utf8",
);

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngTextChunk(keyword, text) {
  const type = Buffer.from("tEXt", "ascii");
  const data = Buffer.concat([Buffer.from(keyword, "ascii"), Buffer.from([0]), Buffer.from(text)]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(Buffer.concat([type, data])), 0);
  return Buffer.concat([length, type, data, checksum]);
}

function preservePngPixelsWithVariantMarker(bytes) {
  let offset = 8;
  while (offset + 12 <= bytes.length) {
    const length = bytes.readUInt32BE(offset);
    const type = bytes.subarray(offset + 4, offset + 8).toString("ascii");
    if (type === "IEND") {
      return Buffer.concat([
        bytes.subarray(0, offset),
        pngTextChunk("cvm", "pwa-test-variant-v2"),
        bytes.subarray(offset),
      ]);
    }
    offset += length + 12;
  }
  throw new Error("unable to find PNG IEND chunk");
}

function preserveSvgPixelsWithVariantMarker(bytes) {
  const source = bytes.toString("utf8");
  const closingTag = "</svg>";
  const closingOffset = source.lastIndexOf(closingTag);
  if (closingOffset < 0) throw new Error("unable to find SVG closing tag");
  return Buffer.from(
    `${source.slice(0, closingOffset)}  <!-- pwa-test-variant-v2 -->\n${source.slice(closingOffset)}`,
    "utf8",
  );
}

async function createV2IconVariants() {
  const manifestPath = path.join(v2Dir, "site.webmanifest");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  const sourceUrls = new Set([
    ...manifest.icons.map((icon) => icon.src),
    ...manifest.shortcuts.flatMap((shortcut) => shortcut.icons.map((icon) => icon.src)),
  ]);
  const replacements = new Map();

  for (const sourceUrl of sourceUrls) {
    const sourceFilename = path.basename(sourceUrl);
    const match = installIconPattern.exec(sourceFilename);
    if (!match) throw new Error(`unexpected install icon path: ${sourceUrl}`);
    const sourceBytes = await readFile(path.join(v2Dir, sourceFilename));
    const variantBytes =
      match[2] === ".png"
        ? preservePngPixelsWithVariantMarker(sourceBytes)
        : preserveSvgPixelsWithVariantMarker(sourceBytes);
    const digest = createHash("sha256").update(variantBytes).digest("hex").slice(0, 12);
    const variantFilename = `${match[1]}-${digest}${match[2]}`;
    await writeFile(path.join(v2Dir, variantFilename), variantBytes);
    replacements.set(sourceUrl, variantFilename);
  }

  for (const icon of manifest.icons) icon.src = replacements.get(icon.src);
  for (const shortcut of manifest.shortcuts) {
    for (const icon of shortcut.icons) icon.src = replacements.get(icon.src);
  }
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

await createV2IconVariants();

const swPath = path.join(v2Dir, "sw.js");
await writeFile(swPath, `${await readFile(swPath, "utf8")}\n// pwa-test-variant-v2\n`, "utf8");

let activeDir = v1Dir;

const contentTypes = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".png", "image/png"],
  [".svg", "image/svg+xml"],
  [".txt", "text/plain; charset=utf-8"],
  [".webmanifest", "application/manifest+json; charset=utf-8"],
]);

function cacheControlFor(filePath) {
  const filename = path.basename(filePath);
  if (["index.html", "site.webmanifest", "sw.js", "version.json"].includes(filename)) {
    return "no-cache, max-age=0, must-revalidate";
  }
  if (
    /^(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[a-f0-9]{12}\.(?:png|svg)$/.test(
      filename,
    )
  ) {
    return "public, max-age=31536000, immutable";
  }
  return "no-store";
}

async function resolveFile(requestPath) {
  const sanitizedPath = requestPath === "/" ? "/index.html" : requestPath;
  const relativePath = sanitizedPath.replace(/^\/+/, "");
  const candidate = path.join(activeDir, relativePath);
  try {
    const details = await stat(candidate);
    if (details.isDirectory()) return path.join(candidate, "index.html");
    return candidate;
  } catch {
    if (!path.extname(relativePath)) {
      return path.join(activeDir, "index.html");
    }
    return null;
  }
}

const server = createServer(async (request, response) => {
  if (!request.url) {
    response.writeHead(400).end("missing url");
    return;
  }

  const url = new URL(request.url, `http://127.0.0.1:${PORT}`);

  if (url.pathname === "/__test/current") {
    response.writeHead(200, { "content-type": "application/json; charset=utf-8" });
    response.end(
      JSON.stringify({ variant: activeDir === v2Dir ? "v2" : "v1", version: nextVersion }),
    );
    return;
  }

  if (url.pathname === "/__test/reset") {
    activeDir = v1Dir;
    response.writeHead(200, { "content-type": "application/json; charset=utf-8" });
    response.end(JSON.stringify({ variant: "v1" }));
    return;
  }

  if (url.pathname === "/__test/switch") {
    activeDir = url.searchParams.get("v") === "2" ? v2Dir : v1Dir;
    response.writeHead(200, { "content-type": "application/json; charset=utf-8" });
    response.end(
      JSON.stringify({ variant: activeDir === v2Dir ? "v2" : "v1", version: nextVersion }),
    );
    return;
  }

  const filePath = await resolveFile(url.pathname);
  if (!filePath) {
    response.writeHead(404).end("not found");
    return;
  }

  const body = await readFile(filePath);
  const ext = path.extname(filePath);
  const headers = {
    "cache-control": cacheControlFor(filePath),
    "content-type": contentTypes.get(ext) ?? "application/octet-stream",
  };
  if (path.basename(filePath) === "sw.js") {
    headers["service-worker-allowed"] = "/";
  }
  response.writeHead(200, headers);
  response.end(body);
});

const cleanup = async () => {
  server.close();
  await rm(tempRoot, { recursive: true, force: true });
};

process.on("SIGINT", () => {
  void cleanup().finally(() => process.exit(0));
});
process.on("SIGTERM", () => {
  void cleanup().finally(() => process.exit(0));
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`PWA test server listening on http://127.0.0.1:${PORT}`);
});
