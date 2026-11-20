// The site's behaviour, as shadcn-vue's components give it: ModeSwitcher, GitHubLink's stars, CommandMenu (⌘K),
// CopyButton, ComponentPreviewTabs' "View Code", DocsTableOfContents' active heading, and the mobile menu.
const root = document.documentElement;
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];

$("#mode")?.addEventListener("click", () => {
  const dark = !root.classList.contains("dark");
  root.classList.toggle("dark", dark);
  root.classList.toggle("light", !dark);
  localStorage.setItem("theme", dark ? "dark" : "light");
});

fetch("https://ungh.cc/repos/nguyenvanduocit/shadcn-rs")
  .then((r) => (r.ok ? r.json() : null))
  .then((data) => {
    const count = data?.repo?.stars;
    const stars = $("#stars");
    if (stars) stars.textContent = !count ? "" : count >= 1000 ? `${(count / 1000).toFixed(1)}k` : count.toLocaleString();
  })
  .catch(() => {});

// CopyButton: the figure's source sits beside its highlighted HTML.
for (const button of $$(".copy-button")) {
  button.addEventListener("click", async () => {
    await navigator.clipboard.writeText(button.parentElement.querySelector("textarea").value);
    button.classList.add("copied");
    setTimeout(() => button.classList.remove("copied"), 2000);
  });
}

// ComponentPreviewTabs: code stays folded to its first lines until asked for.
for (const code of $$(".preview-code")) {
  code.querySelector(".view-code button").addEventListener("click", () => code.setAttribute("data-open", ""));
}

// DocsTableOfContents: the heading being read is the one past 20% of the window.
const tocLinks = $$(".toc a");
if (tocLinks.length) {
  const byId = new Map(tocLinks.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
  const observer = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        for (const a of tocLinks) a.removeAttribute("data-active");
        byId.get(e.target.id)?.setAttribute("data-active", "");
      }
    },
    { rootMargin: "0% 0% -80% 0%" },
  );
  for (const id of byId.keys()) {
    const el = document.getElementById(id);
    if (el) observer.observe(el);
  }
}

// CommandMenu.
const dialog = $("#command");
if (dialog) {
  const input = $("input", dialog);
  const items = $$(".command-item", dialog);
  const groups = $$(".command-group", dialog);
  const empty = $(".command-empty", dialog);
  let selected = 0;
  const visible = () => items.filter((i) => !i.hidden);
  const select = (i) => {
    const v = visible();
    selected = Math.max(0, Math.min(i, v.length - 1));
    for (const item of items) item.removeAttribute("data-selected");
    v[selected]?.setAttribute("data-selected", "");
    v[selected]?.scrollIntoView({ block: "nearest" });
  };
  const filter = () => {
    const q = input.value.trim().toLowerCase();
    for (const item of items) item.hidden = !!q && !item.textContent.toLowerCase().includes(q);
    for (const g of groups) g.hidden = !$$(".command-item", g).some((i) => !i.hidden);
    empty.hidden = visible().length > 0;
    select(0);
  };
  const open = () => {
    input.value = "";
    filter();
    dialog.showModal();
    input.focus();
  };
  $("#search")?.addEventListener("click", open);
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      dialog.open ? dialog.close() : open();
    } else if (e.key === "/" && !dialog.open && !/input|textarea/i.test(document.activeElement?.tagName ?? "")) {
      e.preventDefault();
      open();
    }
  });
  input.addEventListener("input", filter);
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") (e.preventDefault(), select(selected + 1));
    else if (e.key === "ArrowUp") (e.preventDefault(), select(selected - 1));
    else if (e.key === "Enter") visible()[selected]?.click();
  });
  for (const item of items) item.addEventListener("mousemove", () => select(visible().indexOf(item)));
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close();
  });
}

// Mobile menu: the sidebar over the page; on a page without one, the docs.
$("#mobile-menu")?.addEventListener("click", () => {
  if (!$(".docs-sidebar")) {
    location.href = "/docs";
    return;
  }
  document.body.classList.toggle("menu-open");
});
