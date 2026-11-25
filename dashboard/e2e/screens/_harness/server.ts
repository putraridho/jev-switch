// Serves the built UI (`dist/`) for the Playwright screen tests with the exact CSP of
// `tauri.conf.json` as a response header (docs/dashboard/DESIGN.md 8.10): Tauri injects the
// policy only at run time, so a CSP-breaking change has to fail here.
//
// Run: `node e2e/screens/_harness/server.ts [port] [directory]`.

import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";
import tauriConfig from "../../../src-tauri/tauri.conf.json" with { type: "json" };

const root = resolve(
  import.meta.dirname,
  "../../..",
  process.argv[3] ?? "dist",
);
const csp = tauriConfig.app.security.csp;
const port = Number(process.argv[2] ?? "4173");

const contentTypes: Readonly<Record<string, string>> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".png": "image/png",
  ".ico": "image/x-icon",
};

function filePath(url: string): string | null {
  const path = normalize(
    decodeURIComponent(new URL(url, "http://localhost").pathname),
  );
  const file = resolve(join(root, path === "/" ? "index.html" : path));
  return file.startsWith(root) ? file : null;
}

const server = createServer((request, response) => {
  const file = filePath(request.url ?? "/");
  if (file === null) {
    response.writeHead(403).end();
    return;
  }
  readFile(file).then(
    (body) => {
      response.writeHead(200, {
        "Content-Type":
          contentTypes[extname(file)] ?? "application/octet-stream",
        "Content-Security-Policy": csp,
        "Cache-Control": "no-store",
      });
      response.end(body);
    },
    () => {
      response.writeHead(404).end();
    },
  );
});

server.listen(port, "127.0.0.1");
