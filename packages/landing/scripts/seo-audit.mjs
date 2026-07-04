#!/usr/bin/env node
// SEO audit for the built landing site. Run after `astro build`:
//   bun run build && bun run seo:audit
// Checks every marketing page for the on-page signals the content
// strategy depends on (see OPTIMIZED_CONTENT.md). Exits 1 on failure.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const DIST = join(import.meta.dirname, "..", ".vercel", "output", "static");
const SITE = "https://zedra.dev";

// path → required JSON-LD @type values. Canonicals use trailing slashes
// to match @astrojs/sitemap output.
const pages = [
  { path: "/", jsonLd: ["SoftwareApplication", "FAQPage"] },
  { path: "/claude-code", jsonLd: ["FAQPage"] },
  { path: "/codex", jsonLd: ["FAQPage"] },
  { path: "/opencode", jsonLd: ["FAQPage"] },
  { path: "/compare", jsonLd: ["FAQPage"] },
];

const failures = [];
const fail = (page, msg) => failures.push(`${page}: ${msg}`);

const attr = (html, re) => {
  const m = html.match(re);
  return m ? m[1] : null;
};

const decode = (s) =>
  s
    .replaceAll("&amp;", "&")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&#39;", "'");

if (!existsSync(DIST)) {
  console.error(`Build output not found at ${DIST} — run \`bun run build\` first.`);
  process.exit(1);
}

const sitemap = readFileSync(join(DIST, "sitemap-0.xml"), "utf8");
const robots = readFileSync(join(DIST, "robots.txt"), "utf8");
if (!robots.includes(`${SITE}/sitemap-index.xml`)) {
  fail("robots.txt", "missing Sitemap: line pointing at sitemap-index.xml");
}

for (const { path, jsonLd } of pages) {
  const file = join(DIST, path === "/" ? "index.html" : `${path.slice(1)}/index.html`);
  if (!existsSync(file)) {
    fail(path, `built page missing at ${file}`);
    continue;
  }
  const html = readFileSync(file, "utf8");
  const canonicalUrl = path === "/" ? `${SITE}/` : `${SITE}${path}/`;

  const title = attr(html, /<title>([^<]+)<\/title>/);
  if (!title) fail(path, "missing <title>");
  else if (title.length > 65) fail(path, `title ${title.length} chars (max 65): "${title}"`);

  const desc = attr(html, /<meta name="description" content="([^"]*)"/);
  if (!desc) fail(path, "missing meta description");
  else {
    const len = decode(desc).length;
    if (len < 50 || len > 165) fail(path, `meta description ${len} chars (want 50–165)`);
  }

  const canonical = attr(html, /<link rel="canonical" href="([^"]*)"/);
  if (canonical !== canonicalUrl) {
    fail(path, `canonical "${canonical}" ≠ expected "${canonicalUrl}"`);
  }

  for (const tag of ["og:title", "og:description", "og:image", "og:url"]) {
    if (!html.includes(`property="${tag}"`)) fail(path, `missing ${tag}`);
  }
  if (!html.includes('name="twitter:card"')) fail(path, "missing twitter:card");

  const h1Count = (html.match(/<h1[\s>]/g) ?? []).length;
  if (h1Count !== 1) fail(path, `${h1Count} <h1> elements (want exactly 1)`);

  const blocks = [...html.matchAll(/<script type="application\/ld\+json">(.*?)<\/script>/gs)];
  const types = [];
  for (const [, raw] of blocks) {
    try {
      types.push(JSON.parse(raw)["@type"]);
    } catch {
      fail(path, "JSON-LD block does not parse as JSON");
    }
  }
  for (const required of jsonLd) {
    if (!types.includes(required)) {
      fail(path, `missing JSON-LD @type "${required}" (found: ${types.join(", ") || "none"})`);
    }
  }

  if (!sitemap.includes(`<loc>${canonicalUrl}</loc>`)) {
    fail(path, "canonical URL not in sitemap-0.xml");
  }
}

if (failures.length > 0) {
  console.error(`SEO audit: ${failures.length} failure(s)\n`);
  for (const f of failures) console.error(`  ✗ ${f}`);
  process.exit(1);
}
console.log(
  `SEO audit: ${pages.length} pages pass (title, description, canonical, OG, h1, JSON-LD, sitemap).`
);
