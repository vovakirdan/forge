import { createHash } from "node:crypto";
import { lstat, readFile, readdir, writeFile } from "node:fs/promises";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const manifestName = "forge-live-manifest.json";
const assetPath =
  /^assets\/[A-Za-z0-9_-][A-Za-z0-9_.-]*\.(?:js|css|woff2?|png|jpe?g|gif|webp|svg|ico)$/;
const htmlTags: Record<string, readonly string[]> = {
  html: ["lang", "class"],
  head: [],
  meta: ["charset", "name", "content"],
  title: [],
  link: ["rel", "crossorigin", "href"],
  body: [],
  div: ["id"],
  script: ["type", "crossorigin", "src"],
};

// This is a deliberately narrow validator for our generated entry, not a general
// HTML sanitizer. New markup needs an explicit build-boundary change.
export function validateLiveHtml(html: string): string[] {
  const references: string[] = [];
  let scripts = 0;
  const withoutScripts = html.replace(
    /<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi,
    (tag: string, _attributes: string, body: string) => {
      if (body.trim() !== "") throw new Error("Live HTML must not contain inline scripts");
      scripts += 1;
      return tag.replace(body, "");
    },
  );
  if (scripts !== 1) throw new Error("Live HTML requires exactly one external module script");
  const residue = withoutScripts
    .replace(/<!doctype\s+html\s*>/gi, "")
    .replace(
      /<\/?([a-z][a-z0-9]*)([^>]*)>/gi,
      (tag: string, rawName: string, rawAttributes: string) => {
        const name = rawName.toLowerCase();
        const allowed = htmlTags[name];
        if (!allowed) throw new Error("Unsupported live HTML element");
        if (tag.startsWith("</")) {
          if (rawAttributes.trim() !== "") throw new Error("Invalid closing tag");
          return "";
        }
        const values = new Map<string, string>();
        let attributes = rawAttributes.trim().replace(/\/$/, "").trim();
        while (attributes.length > 0) {
          const match = /^([a-zA-Z][a-zA-Z0-9_-]*)(?:\s*=\s*"([^"<>&]*)")?(?:\s+|$)/.exec(
            attributes,
          );
          if (!match?.[1]) throw new Error("Unsupported live HTML attributes");
          const key = match[1].toLowerCase();
          if (!allowed.includes(key) || values.has(key))
            throw new Error("Unsupported live HTML attribute");
          values.set(key, match[2] ?? "");
          attributes = attributes.slice(match[0].length);
        }
        if (name === "script" || name === "link") {
          const reference = values.get(name === "script" ? "src" : "href");
          if (!reference?.startsWith("/") || !assetPath.test(reference.slice(1)))
            throw new Error("Live HTML may reference only local assets");
          if (name === "script" && (values.get("type") !== "module" || !reference.endsWith(".js")))
            throw new Error("Live script must be a local module");
          if (
            name === "link" &&
            !(
              (values.get("rel") === "stylesheet" && reference.endsWith(".css")) ||
              (values.get("rel") === "modulepreload" && reference.endsWith(".js"))
            )
          )
            throw new Error("Unsupported live HTML link");
          references.push(reference.slice(1));
        }
        return "";
      },
    );
  if (/[<>]/.test(residue)) throw new Error("Unrecognized live HTML markup");
  return references;
}

export async function generateLiveManifest(directory: string) {
  const files: { path: string; sha256: string }[] = [];
  let totalBytes = 0;
  let references: string[] | undefined;
  async function visit(relativeDirectory: "" | "assets") {
    for (const name of await readdir(join(directory, relativeDirectory))) {
      const relative = relativeDirectory ? `${relativeDirectory}/${name}` : name;
      const path = join(directory, relative);
      const stat = await lstat(path);
      if (stat.isSymbolicLink()) throw new Error("Live assets must not contain symbolic links");
      if (relative === "assets" && stat.isDirectory()) {
        await visit("assets");
        continue;
      }
      if (!stat.isFile()) throw new Error("Unsupported live asset entry");
      if (relative === manifestName) continue;
      if (relative !== "index.html" && !assetPath.test(relative))
        throw new Error("Unsupported live asset path");
      if (stat.size > 8 * 1024 * 1024)
        throw new Error("Live assets must not exceed 8 MiB per file");
      totalBytes += stat.size;
      if (totalBytes > 32 * 1024 * 1024)
        throw new Error("Live assets must not exceed 32 MiB total");
      if (files.length >= 256) throw new Error("Live assets must not exceed 256 files");
      const body = await readFile(path);
      if (relative === "index.html") references = validateLiveHtml(body.toString("utf8"));
      if (
        extname(relative) === ".css" &&
        /@import\b|url\(\s*["']?(?:https?:|\/\/|data:)/i.test(body.toString("utf8"))
      )
        throw new Error("Live CSS must not load external styles or assets");
      files.push({ path: relative, sha256: createHash("sha256").update(body).digest("hex") });
    }
  }
  await visit("");
  if (!references) throw new Error("Live index.html is missing");
  const paths = new Set(files.map((file) => file.path));
  if (references.some((reference) => !paths.has(reference)))
    throw new Error("A referenced live asset is missing");
  files.sort((left, right) => left.path.localeCompare(right.path, "en"));
  const manifest = { format: "forge-live-v1" as const, files };
  const serialized = `${JSON.stringify(manifest, null, 2)}\n`;
  if (Buffer.byteLength(serialized) > 64 * 1024)
    throw new Error("Live manifest must not exceed 64 KiB");
  await writeFile(join(directory, manifestName), serialized);
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await generateLiveManifest(fileURLToPath(new URL("../dist-live", import.meta.url)));
}
