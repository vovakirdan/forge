import { readFile, realpath, stat } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { extname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const contentTypes: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".ico": "image/x-icon",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".webp": "image/webp",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
};

// This is a local static-hosting proof, not the future authenticated Rust gateway.
const uiRoute =
  /^\/(?:board|team|knowledge|settings|goals|activity|resources|pipelines|chat|(?:tasks|runs|team)\/[A-Za-z0-9_-]+)?\/?$/;
const reservedNamespace = /^\/(?:api|v1|_serverFn)(?:\/|$)/;

function inside(root: string, candidate: string) {
  const path = relative(root, candidate);
  return path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path);
}

function sendText(response: ServerResponse, status: number, text: string, head: boolean) {
  response.writeHead(status, {
    "Content-Type": "text/plain; charset=utf-8",
    "Cache-Control": "no-store",
  });
  response.end(head ? undefined : `${text}\n`);
}

/** Serves existing files only beneath root and a shell for explicitly known UI routes. */
export async function createStaticServer(rootDirectory: string) {
  const root = await realpath(rootDirectory);

  async function findFile(path: string): Promise<string | null> {
    try {
      const candidate = await realpath(resolve(root, `.${path}`));
      if (!inside(root, candidate) || !(await stat(candidate)).isFile()) return null;
      return candidate;
    } catch (error) {
      if (
        error instanceof Error &&
        "code" in error &&
        (error.code === "ENOENT" || error.code === "ENOTDIR")
      )
        return null;
      throw error;
    }
  }

  const shell = await findFile("/_shell.html");
  if (!shell) throw new Error("Static shell missing: run just ui-build-static first");

  async function respond(request: IncomingMessage, response: ServerResponse) {
    const head = request.method === "HEAD";
    if (request.method !== "GET" && !head) {
      response.setHeader("Allow", "GET, HEAD");
      sendText(response, 405, "Method not allowed", head);
      return;
    }
    // Inspect the raw request target before WHATWG URL can erase dot segments.
    let path: string;
    try {
      path = decodeURIComponent((request.url ?? "").split("?")[0] ?? "");
    } catch {
      sendText(response, 400, "Bad request", head);
      return;
    }
    if (
      !path.startsWith("/") ||
      path.startsWith("//") ||
      path.includes("\\") ||
      [...path].some(
        (character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
      ) ||
      path.split("/").some((part) => part === "." || part === "..")
    ) {
      sendText(response, 400, "Bad request", head);
      return;
    }
    if (reservedNamespace.test(path) || path.split("/").some((part) => part.startsWith("."))) {
      sendText(response, 404, "Not found", head);
      return;
    }
    const file = await findFile(path);
    const fallback = uiRoute.test(path) && request.headers.accept?.includes("text/html");
    const target = file ?? (fallback ? shell : null);
    if (!target) {
      sendText(response, 404, "Not found", head);
      return;
    }
    const body = await readFile(target);
    response.writeHead(200, {
      "Content-Type": contentTypes[extname(target)] ?? "application/octet-stream",
      "Content-Length": body.byteLength,
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    });
    response.end(head ? undefined : body);
  }

  return createServer((request, response) => {
    void respond(request, response).catch(() => {
      // Do not echo filesystem paths or request data into the browser response.
      if (!response.headersSent)
        sendText(response, 500, "Static read failed", request.method === "HEAD");
      else response.destroy();
    });
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const root = fileURLToPath(new URL("../dist/client", import.meta.url));
  const server = await createStaticServer(root);
  server.on("error", (error) => {
    console.error(`Static smoke server failed: ${error.message}`);
    process.exitCode = 1;
  });
  server.listen(4174, "127.0.0.1", () => {
    console.log("Static smoke server: http://127.0.0.1:4174 (dist/client only)");
  });
  const stop = () => server.close();
  process.once("SIGTERM", stop);
  process.once("SIGINT", stop);
}
