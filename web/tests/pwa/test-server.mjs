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
    "cache-control": "no-store",
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
