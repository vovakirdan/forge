import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { createHash } from "node:crypto";
import { generateLiveManifest, validateLiveHtml } from "./live-manifest.ts";

const html =
  '<!doctype html><html lang="en"><head><meta charset="UTF-8"><title>Forge</title><script type="module" crossorigin src="/assets/live.js"></script><link rel="stylesheet" crossorigin href="/assets/live.css"></head><body><div id="root"></div></body></html>';

test("live manifest inventories only bounded allowed static files with stable SHA256 hashes", async (context) => {
  const directory = await mkdtemp(join(tmpdir(), "forge-live-manifest-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(join(directory, "assets"));
  await writeFile(join(directory, "index.html"), html);
  await writeFile(join(directory, "assets/live.js"), "export {};\n");
  await writeFile(join(directory, "assets/live.css"), "body{color:white}\n");
  const manifest = await generateLiveManifest(directory);
  assert.equal(manifest.format, "forge-live-v1");
  assert.deepEqual(
    manifest.files.map((file) => file.path),
    ["assets/live.css", "assets/live.js", "index.html"],
  );
  assert.equal(
    manifest.files.find((file) => file.path === "index.html")?.sha256,
    createHash("sha256").update(html).digest("hex"),
  );
  assert.deepEqual(
    JSON.parse(await readFile(join(directory, "forge-live-manifest.json"), "utf8")),
    manifest,
  );
  assert.deepEqual(await generateLiveManifest(directory), manifest);
  await writeFile(join(directory, "secrets.env"), "synthetic-secret");
  await assert.rejects(generateLiveManifest(directory), /Unsupported live asset/);
  await rm(join(directory, "secrets.env"));
  await symlink(join(directory, "index.html"), join(directory, "assets/link.js"));
  await assert.rejects(generateLiveManifest(directory), /symbolic links/);
});

test("live HTML build gate rejects executable inline content, handlers, remote URLs, and inline styles", () => {
  assert.doesNotThrow(() => validateLiveHtml(html));
  for (const unsafe of [
    html.replace('src="/assets/live.js"', 'src="https://example.com/live.js"'),
    html.replace('src="/assets/live.js"', 'src="//example.com/live.js"'),
    html.replace('src="/assets/live.js"', 'src="&#47;assets/live.js"'),
    html.replace("></script>", ">alert(1)</script>"),
    html.replace('id="root"', 'id="root" onclick="alert(1)"'),
    html.replace('id="root"', 'id="root" style="display:none"'),
    html.replace("</head>", "<style>body{display:none}</style></head>"),
    html.replace("</head>", '<base href="https://example.com"></head>'),
    html.replace("</body>", '<iframe src="/assets/live.js"></iframe></body>'),
  ])
    assert.throws(() => validateLiveHtml(unsafe));
});

test("manifest rejects missing referenced assets, excess file sizes, and excess count", async (context) => {
  const directory = await mkdtemp(join(tmpdir(), "forge-live-limits-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(join(directory, "assets"));
  await writeFile(join(directory, "index.html"), html);
  await assert.rejects(generateLiveManifest(directory), /missing/);
  await writeFile(join(directory, "assets/live.js"), "x".repeat(8 * 1024 * 1024 + 1));
  await assert.rejects(generateLiveManifest(directory), /8 MiB/);
  await writeFile(join(directory, "assets/live.js"), "export{};");
  await writeFile(join(directory, "assets/live.css"), "body{}");
  for (let index = 0; index < 254; index += 1)
    await writeFile(join(directory, `assets/extra-${index}.js`), "");
  await assert.rejects(generateLiveManifest(directory), /256 files/);
});
