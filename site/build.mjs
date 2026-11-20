// Builds the website into dist/: the landing page from public/, and the docs as shadcn-vue lays out its own
// (apps/v4/pages/docs at d1bf1b3): a sidebar of sections and components, the page, and "On This Page".
//
// What the pages say comes from the repository, never written here twice:
//   - the docs app's own data, `docs --export` (crates/shadcn/examples/docs/docs/export.rs): every page in the sidebar's
//     order, its title, lead and notes, each demo's code as the app's Code tab cuts it, and the last verdicts;
//   - each demo's interaction, an animated WebP motion.py cuts from its page's script (`docs --shots`);
//   - each other demo's preview, the frame the verdict rendered (`<page>/<Demo>/ours.png` under /tmp/shadcn-verdict[-dark]/all,
//     from `crates/shadcn/verdict/run.sh` and `run.sh --dark`);
//   - README.md, crates/shadcn/README.md and docs/adr/*.md, as Markdown.
//
//   uv run --with pillow python motion.py <shots> .cache/motion/<light|dark> <light|dark>   # first, for the motion
//   bun build.mjs            # export, convert, render
//   bun build.mjs --no-cargo # reuse .cache/docs.json

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked, Renderer } from "marked";
import { createHighlighter } from "shiki";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const dist = join(here, "dist");
const cache = join(here, ".cache");
const REPO = "https://github.com/nguyenvanduocit/shadcn-rs";
const BLOB = `${REPO}/blob/master`;
const VUE_REGISTRY = "https://github.com/unovue/shadcn-vue/tree/dev/apps/v4/registry/new-york-v4";
const VERDICT = { light: "/tmp/shadcn-verdict/all", dark: "/tmp/shadcn-verdict-dark/all" };

// --- Data -------------------------------------------------------------------------------------------------------

mkdirSync(cache, { recursive: true });
const exported = join(cache, "docs.json");
if (!process.argv.includes("--no-cargo")) {
  execFileSync("cargo", ["run", "-q", "--release", "-p", "shadcn", "--example", "docs", "--", "--export", exported], {
    cwd: root,
    stdio: "inherit",
  });
}
const docs = JSON.parse(readFileSync(exported, "utf8"));

const highlighter = await createHighlighter({
  themes: ["github-light", "github-dark"],
  langs: ["rust", "toml", "sh", "ts", "json", "vue", "html", "css"],
});
const highlight = (code, lang) =>
  highlighter.codeToHtml(code, {
    lang: highlighter.getLoadedLanguages().includes(lang) ? lang : "text",
    themes: { light: "github-light", dark: "github-dark" },
    defaultColor: false,
  });

const escape = (s) =>
  String(s).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
const slug = (s) =>
  String(s)
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

// --- Markdown ---------------------------------------------------------------------------------------------------

/** Links between the repository's own Markdown files go to their pages here; anything else in the repo to GitHub. */
function link(href, from) {
  if (/^[a-z]+:/.test(href) || href.startsWith("#") || href.startsWith("/")) return href;
  const adr = href.match(/^(?:\.\.\/)*(?:docs\/adr\/)?(\d{4})-[^/]*\.md(#.*)?$/);
  if (adr) return `/docs/decisions/${adr[1]}${adr[2] ?? ""}`;
  return `${BLOB}/${join(dirname(from), href)}`;
}

function markdown(md, from) {
  const toc = [];
  const marked = new Marked({
    renderer: {
      heading({ tokens, depth }) {
        const html = this.parser.parseInline(tokens);
        const id = slug(html);
        if (depth === 2 || depth === 3) toc.push({ id, text: html.replace(/<[^>]+>/g, ""), depth });
        return `<h${depth} id="${id}">${html}</h${depth}>\n`;
      },
      code({ text, lang }) {
        return codeFigure(text, lang || "text");
      },
      link({ href, title, tokens }) {
        return `<a href="${escape(link(href, from))}"${title ? ` title="${escape(title)}"` : ""}>${this.parser.parseInline(tokens)}</a>`;
      },
      table(token) {
        return `<div class="prose-table">${Renderer.prototype.table.call(this, token)}</div>`;
      },
    },
  });
  return { html: marked.parse(intraDoc(md)), toc };
}
// Rustdoc's intra-doc links (`[`DialogState`]`) name an item the site has no page for: the code alone.
const intraDoc = (md) => (md ?? "").replace(/\[(`[^`]+`)\](?![(\[])/g, "$1");
const inline = (md) => new Marked().parseInline(intraDoc(md));

function codeFigure(code, lang) {
  return `<figure data-pretty-code-figure><button class="copy-button" data-slot="copy-button" aria-label="Copy">${ICON.copy}${ICON.check}</button>${highlight(code.replace(/\n$/, ""), lang)}<textarea hidden>${escape(code)}</textarea></figure>`;
}

/** A Markdown file split at its first H1: its title, its first paragraph (the description) and the rest. */
function article(path) {
  const md = readFileSync(join(root, path), "utf8");
  const [, title = "", rest = md] = md.match(/^#\s+(.*)\n([\s\S]*)$/) ?? [];
  const body = rest.trim();
  const [first, ...more] = body.split(/\n\n/);
  return { title, description: first.startsWith("#") ? "" : first.replace(/\n/g, " "), body: first.startsWith("#") ? body : more.join("\n\n") };
}

/** One `## heading` section of a Markdown file, without its heading. */
function section(path, heading) {
  const md = readFileSync(join(root, path), "utf8");
  const start = md.indexOf(`\n## ${heading}\n`);
  if (start < 0) throw new Error(`${path} has no "## ${heading}"`);
  const from = start + heading.length + 5;
  const end = md.indexOf("\n## ", from);
  return md.slice(from, end < 0 ? undefined : end).trim();
}

// --- Demo previews ----------------------------------------------------------------------------------------------

/** A PNG's size in pixels, from its IHDR. */
function pngSize(path) {
  const b = readFileSync(path);
  return { width: b.readUInt32BE(16), height: b.readUInt32BE(20) };
}

/** Each demo's interaction where its page's script drives one: motion.py's animated WebP, light and dark. */
const MOTION = Object.fromEntries(
  ["light", "dark"].map((mode) => {
    const manifest = join(cache, "motion", mode, "motion.json");
    return [mode, existsSync(manifest) ? JSON.parse(readFileSync(manifest, "utf8")) : {}];
  }),
);

/** Each demo's preview, light and dark, under dist/demos/<page>/, with its size at 1×: the animation of what its
 *  script does when there is one, else `ours.png`, the still the verdict judged. */
function preview(page, file) {
  const demo = file.replace(/\.vue$/, "");
  const out = {};
  for (const mode of ["light", "dark"]) {
    const motion = MOTION[mode][page]?.[file];
    if (motion) {
      const name = `${demo}.motion${mode === "dark" ? "-dark" : ""}.webp`;
      const target = join(dist, "demos", page, name);
      mkdirSync(dirname(target), { recursive: true });
      cpSync(join(cache, "motion", mode, page, `${demo}.webp`), target);
      out[mode] = { src: `/demos/${page}/${name}`, width: motion.width, height: motion.height, motion: true };
      continue;
    }
    const src = join(VERDICT[mode], page, demo, "ours.png");
    if (!existsSync(src)) continue;
    const name = `${demo}${mode === "dark" ? "-dark" : ""}.webp`;
    const target = join(dist, "demos", page, name);
    const cached = join(cache, "demos", page, name);
    mkdirSync(dirname(target), { recursive: true });
    mkdirSync(dirname(cached), { recursive: true });
    if (!existsSync(cached) || statSync(cached).mtimeMs < statSync(src).mtimeMs) {
      execFileSync("cwebp", ["-quiet", "-q", "90", src, "-o", cached]);
    }
    cpSync(cached, target);
    const { width, height } = pngSize(src);
    out[mode] = { src: `/demos/${page}/${name}`, width: Math.round(width / 2), height: Math.round(height / 2) };
  }
  return out;
}

// --- Navigation ---------------------------------------------------------------------------------------------------

const components = docs.pages.filter((p) => p.group === "Components");
const guides = docs.pages.filter((p) => p.group !== "Components");
const componentHref = (p) => `/docs/components/${p.name}`;
const adrs = readdirSync(join(root, "docs/adr"))
  .filter((f) => /^\d{4}-.*\.md$/.test(f))
  .sort()
  .map((f) => ({ file: `docs/adr/${f}`, number: f.slice(0, 4), ...article(`docs/adr/${f}`) }));

const SECTIONS = [
  { name: "Introduction", href: "/docs" },
  { name: "Components", href: "/docs/components" },
  { name: "Installation", href: "/docs/installation" },
  { name: "Theming", href: "/docs/theming" },
  { name: "Fidelity", href: "/docs/fidelity" },
  { name: "Porting", href: "/docs/porting" },
  { name: "Decisions", href: "/docs/decisions" },
];
const NAV = [
  { label: "Docs", href: "/docs" },
  { label: "Components", href: "/docs/components" },
  { label: "Theming", href: "/docs/theming" },
  { label: "Fidelity", href: "/docs/fidelity" },
  { label: "Decisions", href: "/docs/decisions" },
];

/** Every docs page in reading order, for previous / next. */
const ORDER = [
  ...SECTIONS.map((s) => ({ title: s.name, href: s.href })),
  ...guides.map((p) => ({ title: p.title, href: componentHref(p) })),
  ...components.map((p) => ({ title: p.title, href: componentHref(p) })),
  ...adrs.map((a) => ({ title: a.title, href: `/docs/decisions/${a.number}` })),
];

// --- Layout -------------------------------------------------------------------------------------------------------

const ICON = {
  logo: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z"/><path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12"/><path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/></svg>`,
  github: `<svg viewBox="0 0 438.549 438.549"><path fill="currentColor" d="${readFileSync(join(here, "github-path.txt"), "utf8").trim()}"/></svg>`,
  mode: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path stroke="none" d="M0 0h24v24H0z" fill="none"/><path d="M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0"/><path d="M12 3l0 18"/><path d="M12 9l4.65 -4.65"/><path d="M12 14.3l7.37 -7.37"/><path d="M12 19.6l8.85 -8.85"/></svg>`,
  terminal: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19h8"/><path d="m4 17 6-6-6-6"/></svg>`,
  copy: `<svg class="i-copy" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>`,
  check: `<svg class="i-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>`,
  left: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l14 0"/><path d="M5 12l6 6"/><path d="M5 12l6 -6"/></svg>`,
  right: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l14 0"/><path d="M13 18l6 -6"/><path d="M13 6l6 6"/></svg>`,
  external: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 7l-10 10"/><path d="M8 7l9 0l0 9"/></svg>`,
  menu: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 8h16"/><path d="M4 16h16"/></svg>`,
  search: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>`,
  pass: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/></svg>`,
  file: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>`,
};

function header(path) {
  // The most specific item that holds the page is the active one: Components on a component's page, not Docs too.
  const holds = (href) => path === href || path.startsWith(`${href}/`);
  const best = NAV.filter((n) => holds(n.href)).sort((a, b) => b.href.length - a.href.length)[0]?.href;
  const active = (href) => (href === best ? " data-active" : "");
  return `<header class="site-header">
    <div class="container-wrapper px-6">
      <div class="header-row">
        <button class="btn btn-ghost btn-icon mobile-menu-trigger" id="mobile-menu" aria-label="Menu">${ICON.menu}</button>
        <nav class="main-nav">
          <a class="btn btn-ghost btn-icon logo" href="/" aria-label="shadcn-rs">${ICON.logo}</a>
          ${NAV.map((n) => `<a class="btn btn-ghost btn-sm"${active(n.href)} href="${n.href}">${n.label}</a>`).join("\n          ")}
        </nav>
        <div class="header-actions">
          <button class="search-trigger" id="search" type="button">
            <span class="search-long">Search documentation...</span><span class="search-short">Search...</span>
            <kbd class="kbd-group"><kbd>⌘</kbd><kbd>K</kbd></kbd>
          </button>
          <span class="separator hide-md"></span>
          <a class="btn btn-ghost btn-sm github" href="${REPO}" target="_blank" rel="noreferrer">${ICON.github}<span class="stars" id="stars"></span></a>
          <span class="separator"></span>
          <button class="btn btn-ghost btn-icon" id="mode" title="Toggle theme">${ICON.mode}<span class="sr-only">Toggle theme</span></button>
          <span class="separator hide-sm"></span>
          <a class="btn btn-primary btn-sm btn-31 hide-sm" href="/docs/installation">${ICON.terminal} Install</a>
        </div>
      </div>
    </div>
  </header>`;
}

const FOOTER = `<footer class="site-footer">
    <div class="container-wrapper px-4">
      <div class="footer-row">
        <div class="footer-text">
          Every component on this site was rendered by shadcn-rs, not by a browser. Built by
          <a href="https://aiocean.io" target="_blank" rel="noreferrer">aiocean</a>. The source code is available on
          <a href="${REPO}" target="_blank" rel="noreferrer">GitHub</a>.
        </div>
      </div>
    </div>
  </footer>`;

function head(title, description) {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)} - shadcn-rs</title>
<meta name="description" content="${escape(description)}">
<meta property="og:title" content="${escape(title)}">
<meta property="og:description" content="${escape(description)}">
<meta property="og:image" content="https://shadcn-rs.aiocean.io/shots/docs-light.webp">
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="preload" href="/fonts/Geist.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="/style.css">
<script>
  (() => {
    const saved = localStorage.getItem("theme");
    const dark = saved ? saved === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.classList.add(dark ? "dark" : "light");
  })();
</script>
</head>`;
}

function sidebar(path) {
  const item = (title, href, active) =>
    `<li><a class="sidebar-button"${active ? " data-active" : ""} href="${href}">${escape(title)}</a></li>`;
  const sectionActive = (href) => (href === "/docs" ? path === "/docs" : path === href || (path.startsWith(`${href}/`) && href !== "/docs/components"));
  const group = (label, items) =>
    `<div class="sidebar-group"><div class="sidebar-label">${label}</div><ul class="sidebar-menu">${items.join("")}</ul></div>`;
  return `<aside class="docs-sidebar"><div class="sidebar-content">
    ${group("Sections", SECTIONS.map((s) => item(s.name, s.href, sectionActive(s.href) || (s.href === "/docs/components" && path === s.href))))}
    ${guides.length ? group("Getting Started", guides.map((p) => item(p.title, componentHref(p), path === componentHref(p)))) : ""}
    ${group("Components", components.map((p) => item(p.title, componentHref(p), path === componentHref(p))))}
    ${group("Decisions", adrs.map((a) => item(a.title.replace(/^\d{4}\.\s*/, ""), `/docs/decisions/${a.number}`, path === `/docs/decisions/${a.number}`)))}
    <div class="sidebar-fade"></div>
  </div></aside>`;
}

function neighbours(path) {
  const i = ORDER.findIndex((o) => o.href === path);
  return { prev: i > 0 ? ORDER[i - 1] : null, next: i >= 0 && i < ORDER.length - 1 ? ORDER[i + 1] : null };
}

/** A docs page: sidebar, title and description, the body, previous / next, and "On This Page". */
function docsPage({ path, title, description, body, toc = [], links = "" }) {
  const { prev, next } = neighbours(path);
  const arrow = (n, dir) =>
    n ? `<a class="btn btn-secondary btn-icon-7" href="${n.href}" title="${escape(n.title)}">${ICON[dir]}<span class="sr-only">${dir === "left" ? "Previous" : "Next"}</span></a>` : "";
  const pager = (n, dir) =>
    n
      ? `<a class="btn btn-secondary btn-sm pager-${dir}" href="${n.href}">${dir === "left" ? ICON.left : ""}${escape(n.title)}${dir === "right" ? ICON.right : ""}</a>`
      : "<span></span>";
  return `${head(title, description.replace(/<[^>]+>/g, ""))}
<body>
<div data-slot="layout" class="layout">
  ${header(path)}
  <main>
    <div class="container-wrapper docs-wrapper">
      <div class="docs-grid">
        ${sidebar(path)}
        <div class="docs-page" data-slot="docs">
          <div class="docs-main">
            <div class="docs-column">
              <div class="docs-title">
                <div class="docs-title-row">
                  <h1>${escape(title)}</h1>
                  <div class="docs-nav">${arrow(prev, "left")}${arrow(next, "right")}</div>
                </div>
                ${description ? `<p class="docs-description">${description}</p>` : ""}
              </div>
              ${links ? `<div class="docs-links">${links}</div>` : ""}
              <div class="prose">${body}</div>
              <div class="docs-pager">${pager(prev, "left")}${pager(next, "right")}</div>
            </div>
          </div>
          <div class="docs-toc">
            ${toc.length ? `<div class="toc"><p class="toc-title">On This Page</p>${toc.map((t) => `<a href="#${t.id}" data-depth="${t.depth}">${escape(t.text)}</a>`).join("")}</div>` : ""}
          </div>
        </div>
      </div>
    </div>
  </main>
  ${FOOTER}
</div>
${palette()}
<script src="/main.js" defer></script>
</body>
</html>`;
}

/** The command menu ⌘K opens: every page, grouped as the sidebar groups them. */
function palette() {
  const groups = [
    ["Sections", SECTIONS.map((s) => [s.name, s.href])],
    ["Getting Started", guides.map((p) => [p.title, componentHref(p)])],
    ["Components", components.map((p) => [p.title, componentHref(p)])],
    ["Decisions", adrs.map((a) => [a.title, `/docs/decisions/${a.number}`])],
  ];
  return `<dialog class="command-dialog" id="command">
  <div class="command">
    <div class="command-input">${ICON.search}<input placeholder="Search documentation..." autocomplete="off" spellcheck="false"></div>
    <div class="command-list">
      ${groups
        .filter(([, items]) => items.length)
        .map(([label, items]) => `<div class="command-group"><div class="command-group-heading">${label}</div>${items.map(([t, h]) => `<a class="command-item" href="${h}">${ICON.file}<span>${escape(t)}</span></a>`).join("")}</div>`)
        .join("")}
      <p class="command-empty" hidden>No results found.</p>
    </div>
  </div>
</dialog>`;
}

function write(path, html) {
  const file = path === "/" ? join(dist, "index.html") : join(dist, path.slice(1), "index.html");
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, html);
}

// --- Pages --------------------------------------------------------------------------------------------------------

rmSync(dist, { recursive: true, force: true });
cpSync(join(here, "public"), dist, { recursive: true });

// The landing page keeps its own body; its header is the docs' header.
{
  const landing = readFileSync(join(here, "public/index.html"), "utf8");
  const replaced = landing.replace(/<header class="site-header">[\s\S]*?<\/header>/, header("/")).replace("</body>", `${palette()}\n</body>`);
  writeFileSync(join(dist, "index.html"), replaced);
}

const grade = (j, mode) =>
  j ? `<span class="badge badge-grade" data-grade="${j.grade}">${ICON.pass}${mode} ${j.grade} ${Math.round(j.score)}</span>` : "";

// Components and guides.
let judged = 0;
for (const p of docs.pages) {
  const path = componentHref(p);
  const demos = p.demos.map((d) => {
    const shots = preview(p.name, d.file);
    const img = (mode, cls) =>
      shots[mode]
        ? `<img class="${cls}" src="${shots[mode].src}" width="${shots[mode].width}" height="${shots[mode].height}" alt="${escape(`${p.title}: ${d.title}`)}" loading="lazy">`
        : "";
    if (d.light || d.dark) judged += 1;
    const id = slug(d.title);
    return {
      toc: { id, text: d.title, depth: 2 },
      html: `<h2 id="${id}" class="demo-heading"><span>${escape(d.title)}</span> <code class="demo-file">${escape(d.file)}</code><span class="demo-grades">${grade(d.light, "Light")}${grade(d.dark, "Dark")}</span></h2>
<div data-slot="component-preview" class="component-preview">
  <div class="preview">${shots.dark ? `${img("light", "only-light")}${img("dark", "only-dark")}` : img("light", "")}</div>
  ${d.code ? `<div data-slot="code" class="preview-code"><div class="preview-code-inner">${codeFigure(d.code, "rust")}<div class="view-code"><div class="view-code-fade"></div><button type="button" class="btn btn-outline btn-sm">View Code</button></div></div></div>` : ""}
</div>`,
    };
  });
  const notes = p.notes.map((n) => `<p>${inline(n)}</p>`).join("\n");
  const links = [
    p.registry ? `<a class="badge badge-secondary" href="${VUE_REGISTRY}/${p.registry}" target="_blank" rel="noreferrer">shadcn-vue ${escape(p.registry)} ${ICON.external}</a>` : "",
    `<a class="badge badge-secondary" href="${BLOB}/${p.path}" target="_blank" rel="noreferrer">${escape(p.path.split("/").pop())} ${ICON.external}</a>`,
    existsSync(join(root, "crates/shadcn/src", `${p.name.replaceAll("-", "_")}.rs`))
      ? `<a class="badge badge-secondary" href="${BLOB}/crates/shadcn/src/${p.name.replaceAll("-", "_")}.rs" target="_blank" rel="noreferrer">src/${p.name.replaceAll("-", "_")}.rs ${ICON.external}</a>`
      : "",
  ].join("");
  write(
    path,
    docsPage({
      path,
      title: p.title,
      description: inline(p.lead),
      links,
      body: `${notes}\n${demos.map((d) => d.html).join("\n")}`,
      toc: demos.map((d) => d.toc),
    }),
  );
}

// Introduction: the root README.
{
  const a = article("README.md");
  const { html, toc } = markdown(a.body, "README.md");
  write("/docs", docsPage({ path: "/docs", title: "Introduction", description: inline(a.description), body: html, toc }));
}

// Installation: how to run it and depend on it.
{
  const md = `## Run the docs

The docs app is this site as a native window: every component, its demos and their code, light and dark.

\`\`\`sh
git clone ${REPO}
cd shadcn-rs
cargo run --release -p shadcn --example docs
\`\`\`

## Add it to a project

${section("README.md", "Using it")}

## Your first window

\`\`\`rust
use gui::prelude::*;
use gui::{RunOptions, Signal, div};
use shadcn::*;

fn main() {
    provide_theme(Signal::new(Theme::light()));
    let airplane = Signal::new(false);
    gui::run(
        "Settings",
        move || {
            card()
                .max_w_px(384.0)
                .child(card_header().child(card_title("Network")))
                .child(
                    card_content().child(
                        div()
                            .flex()
                            .items_center()
                            .gap(2.0)
                            .child(switch(&airplane))
                            .child(label("Airplane Mode")),
                    ),
                )
                .into_element()
        },
        RunOptions::default(),
    )
}
\`\`\`

State lives in \`Signal\`s you create once; the closure rebuilds the element tree from them each frame. Each component
is a function returning a \`gui\` element, styled after it as Vue's \`class\` prop is: see [Porting](/docs/porting).

## Checking it

${section("README.md", "Checking it")}`;
  const { html, toc } = markdown(md, "README.md");
  write(
    "/docs/installation",
    docsPage({
      path: "/docs/installation",
      title: "Installation",
      description: "Run the docs app, then depend on the crates from Git.",
      body: html,
      toc,
    }),
  );
}

// Porting: the shadcn crate's README.
{
  const a = article("crates/shadcn/README.md");
  const { html, toc } = markdown(a.body, "crates/shadcn/README.md");
  write(
    "/docs/porting",
    docsPage({ path: "/docs/porting", title: "Porting", description: inline(a.description), body: html, toc }),
  );
}

// Components: the list, as shadcn-vue's ComponentsList.
{
  const list = `<div class="components-list">${components.map((p) => `<a href="${componentHref(p)}">${escape(p.title)}</a>`).join("")}</div>`;
  const guidesList = guides.length
    ? `<h2 id="guides">Getting Started</h2><div class="components-list">${guides.map((p) => `<a href="${componentHref(p)}">${escape(p.title)}</a>`).join("")}</div>`
    : "";
  write(
    "/docs/components",
    docsPage({
      path: "/docs/components",
      title: "Components",
      description: `Every component of shadcn-vue's new-york-v4 registry, and the pieces it builds for AI apps: ${components.length} pages, each with its demos as shadcn-rs renders them and the code that renders them.`,
      body: `${list}${guidesList}`,
    }),
  );
}

// Theming: every token, light and dark.
{
  const swatch = (value, mode) =>
    `<div class="swatch" data-mode="${mode}"><span class="swatch-chip" style="background:${value}"></span><code>${escape(value)}</code></div>`;
  const rows = docs.tokens.map((t) => `<tr><td><code>--${escape(t.name)}</code></td><td>${swatch(t.light, "light")}</td><td>${swatch(t.dark, "dark")}</td></tr>`).join("");
  const md = `Components read the theme's tokens, never a colour of their own: \`let t = theme();\` then \`.bg(t.primary)\`,
\`.text_color(t.muted_foreground)\`, \`.border_color(t.border)\`. \`Theme::light()\` and \`Theme::dark()\` are shadcn's
neutral base colour; \`provide_theme\` sets the one a window starts with, and setting its \`Signal\` switches every
component at once.

\`\`\`rust
let theme = Signal::new(Theme::light());
provide_theme(theme.clone());
// later, from a button:
theme.set(Theme::dark());
\`\`\`

## Tokens`;
  const { html } = markdown(md, "crates/shadcn/src/theme.rs");
  write(
    "/docs/theming",
    docsPage({
      path: "/docs/theming",
      title: "Theming",
      description: "shadcn's CSS variables, as the fields of one Rust struct.",
      body: `${html}<div class="prose-table"><table class="tokens-table"><thead><tr><th>Token</th><th>Light</th><th>Dark</th></tr></thead><tbody>${rows}</tbody></table></div>`,
      toc: [{ id: "tokens", text: "Tokens", depth: 2 }],
    }),
  );
}

// Fidelity: every demo's last verdict.
{
  const all = docs.pages.flatMap((p) => p.demos.map((d) => ({ page: p, demo: d })));
  const stats = (mode) => {
    const js = all.map((x) => x.demo[mode]).filter(Boolean);
    const count = (g) => js.filter((j) => j.grade === g).length;
    const mean = js.length ? js.reduce((s, j) => s + j.score, 0) / js.length : 0;
    return { n: js.length, mean, pass: count("PASS"), better: count("BETTER"), warn: count("WARN"), fail: count("FAIL") };
  };
  const card = (value, label) => `<div class="stat-card"><div class="stat-value">${value}</div><div class="stat-label">${label}</div></div>`;
  const summary = ["light", "dark"]
    .map((m) => {
      const s = stats(m);
      const M = m[0].toUpperCase() + m.slice(1);
      return `<div class="stat-grid">${card(s.mean.toFixed(1), `${M}: mean of ${s.n} demos`)}${card(s.pass, `${M}: PASS`)}${card(s.better, `${M}: BETTER`)}${card(s.warn, `${M}: WARN`)}${card(s.fail, `${M}: FAIL`)}</div>`;
    })
    .join("");
  const rows = all
    .map(
      ({ page, demo }) =>
        `<tr><td><a href="${componentHref(page)}#${slug(demo.title)}">${escape(page.title)}</a></td><td><code>${escape(demo.file)}</code></td><td>${grade(demo.light, "")}</td><td>${grade(demo.dark, "")}</td></tr>`,
    )
    .join("");
  const intro = markdown(section("crates/shadcn/README.md", "Judging it against shadcn-vue"), "crates/shadcn/README.md").html;
  write(
    "/docs/fidelity",
    docsPage({
      path: "/docs/fidelity",
      title: "Fidelity",
      description:
        "How close each demo comes to shadcn-vue's own. The verdict lays our render over Chrome's, cropped to the ink, and scores the share that matches: PASS from 90, WARN from 75, FAIL below; BETTER differs on purpose.",
      body: `<h2 id="summary">Summary</h2>${summary}<h2 id="by-demo">By demo</h2><div class="prose-table"><table><thead><tr><th>Component</th><th>Demo</th><th>Light</th><th>Dark</th></tr></thead><tbody>${rows}</tbody></table></div><h2 id="how-it-is-judged">How it is judged</h2>${intro}`,
      toc: [
        { id: "summary", text: "Summary", depth: 2 },
        { id: "by-demo", text: "By demo", depth: 2 },
        { id: "how-it-is-judged", text: "How it is judged", depth: 2 },
      ],
    }),
  );
}

// Decisions: the ADR index and each record.
{
  const a = article("docs/adr/README.md");
  // The README's table reads as a list here: number, title and status over the decision, which a phone fits.
  const rows = new Map(
    [...a.body.matchAll(/^\| \[(\d{4})\]\([^)]*\) \| ([^|]+) \| (.+) \|$/gm)].map((m) => [m[1], { status: m[2].trim(), decision: m[3] }]),
  );
  const list = adrs
    .map((adr) => {
      const r = rows.get(adr.number) ?? { status: "", decision: adr.description };
      return `<a class="adr-item" href="/docs/decisions/${adr.number}">
        <div class="adr-head"><span class="adr-number">${adr.number}</span><span class="adr-title">${inline(adr.title.replace(/^\d{4}\.\s*/, ""))}</span>${r.status ? `<span class="badge badge-outline">${escape(r.status)}</span>` : ""}</div>
        <p class="adr-decision">${inline(r.decision)}</p>
      </a>`;
    })
    .join("\n");
  write(
    "/docs/decisions",
    docsPage({ path: "/docs/decisions", title: "Decisions", description: inline(a.description), body: `<div class="adr-list">${list}</div>` }),
  );
  for (const adr of adrs) {
    const path = `/docs/decisions/${adr.number}`;
    const { html, toc } = markdown(adr.body, adr.file);
    write(
      path,
      docsPage({
        path,
        title: adr.title,
        description: inline(adr.description),
        body: html,
        toc,
        links: `<a class="badge badge-secondary" href="${BLOB}/${adr.file}" target="_blank" rel="noreferrer">${escape(adr.file)} ${ICON.external}</a>`,
      }),
    );
  }
}

// Not found.
writeFileSync(
  join(dist, "404.html"),
  docsPage({
    path: "/404",
    title: "Page not found",
    description: "There is no page here. The sidebar lists every page; ⌘K searches them.",
    body: `<p><a href="/docs">Back to the docs</a></p>`,
  }),
);

console.log(`built ${docs.pages.length} component pages (${judged} demos), ${adrs.length} decisions, into ${dist}`);
