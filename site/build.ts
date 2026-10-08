// Builds the static site into site/dist.
//
// Sources:
//   - content/*.md            hand-written pages (overview, design approach)
//   - Obsidian vault          lore and design notes; the Primer note is the index
//   - ../specs/*.md           design specs
//   - git log                 devlog
//
// site/dist/play (Godot web export) and site/dist/map data (CLI export) are
// produced by their own commands and left untouched here. See CLAUDE.md.

import { execFileSync } from "node:child_process";
import {
	cpSync,
	existsSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import MarkdownIt from "markdown-it";

const SITE_DIR = import.meta.dir;
const REPO_DIR = join(SITE_DIR, "..");
const OUT_DIR = join(SITE_DIR, "dist");
const TERRAIN_WASM = join(
	REPO_DIR,
	"gdextension/target/wasm32-unknown-unknown/release/mg_web.wasm",
);
export const VAULT_DIR =
	process.env.MG_VAULT_DIR ??
	join(homedir(), "Documents/mop-jones-brain/Notes");

export const NOTE_PREFIX = "Margin's Grip - ";
export const PRIMER_NOTE = "Margin's Grip Game World Primer.md";
/** Primer groups shown under Lore. Every other group goes under Design. */
const LORE_GROUPS = ["World", "Quests"];
/** Primer groups written for the earlier Bevy prototype. */
const PROTOTYPE_ERA_GROUPS = ["Technical"];
const SUMMARY_MAX_LENGTH = 160;
const CONTENTS_MAX_ENTRIES = 18;

const NAV = [
	{ label: "Overview", url: "/" },
	{ label: "Map", url: "/map/" },
	{ label: "Lore", url: "/lore/" },
	{ label: "Design", url: "/design/" },
	{ label: "Devlog", url: "/devlog/" },
];

type Section = "lore" | "design";

export interface Note {
	title: string;
	group: string;
	section: Section;
	url: string;
	markdown: string;
	/** Titles of published notes this one links to. */
	linksTo: Set<string>;
	/** Titles this one links to that have no note yet: stubs. */
	stubs: Set<string>;
}

/** Everything the wiki sidebar and search need, built once. */
interface Wiki {
	notes: Note[];
	notesByTitle: Map<string, Note>;
	specs: Spec[];
	/** Note title -> titles of the notes that link to it. */
	backlinks: Map<string, string[]>;
}

interface Spec {
	id: string;
	title: string;
	url: string;
	markdown: string;
}

interface Commit {
	date: string;
	subject: string;
}

interface DevlogEntry {
	date: string;
	changes: string[];
}

// ─── Text helpers ────────────────────────────────────────────────────────────

export const slugify = (text: string): string =>
	text
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-|-$/g, "");

const escapeHtml = (text: string): string =>
	text
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;")
		.replace(/"/g, "&quot;");

const FRONTMATTER = /^---\n[\s\S]*?\n---\n/;
const stripFrontmatter = (markdown: string): string =>
	markdown.replace(FRONTMATTER, "");
/** The frontmatter block at the top of a note, or "" if it has none. */
export const frontmatterOf = (markdown: string): string =>
	markdown.match(FRONTMATTER)?.[0] ?? "";

const stripLeadingTitle = (markdown: string): string =>
	markdown.replace(/^\s*# .*\n/, "");

/** First sentence of the first prose paragraph, as plain text. */
function summarise(markdown: string): string {
	const paragraph = markdown
		.split("\n")
		.map((line) => line.trim())
		.find((line) => line.length > 0 && /^[A-Za-z*_[]/.test(line));
	if (!paragraph) return "";
	const plain = paragraph
		.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
		.replace(/[*_`]/g, "");
	const sentence = plain.match(/^.*?[.!?](?=\s|$)/)?.[0] ?? plain;
	return sentence.length > SUMMARY_MAX_LENGTH
		? `${sentence.slice(0, SUMMARY_MAX_LENGTH - 1).trimEnd()}…`
		: sentence;
}

// ─── Markdown ────────────────────────────────────────────────────────────────

const markdownRenderer = new MarkdownIt({ linkify: true });
// Heading ids, so Obsidian links to a heading ([[Note#Heading]]) resolve.
markdownRenderer.renderer.rules.heading_open = (
	tokens,
	index,
	options,
	_env,
	self,
) => {
	tokens[index].attrSet("id", slugify(tokens[index + 1].content));
	return self.renderToken(tokens, index, options);
};

const WIKILINK = /!?\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g;

/**
 * Whether a wikilink target is a note of this world rather than something
 * else in the vault (a journal day, a category).
 */
const isWorldNote = (target: string): boolean =>
	target.startsWith(NOTE_PREFIX) || !target.includes("/");

/**
 * Turn Obsidian wikilinks into markdown links. A link to a world note that
 * is not written yet becomes a stub, shown as such and openable in the
 * editor; links elsewhere in the vault (journal entries, categories) become
 * plain text.
 */
function resolveWikilinks(
	markdown: string,
	notesByTitle: Map<string, Note>,
): string {
	return markdown.replace(
		WIKILINK,
		(_match, target: string, heading?: string, alias?: string) => {
			const title = target.trim().replace(NOTE_PREFIX, "");
			const label = alias ?? heading ?? title;
			const note = notesByTitle.get(title);
			if (note) {
				const anchor = heading ? `#${slugify(heading)}` : "";
				return `[${label}](${note.url}${anchor})`;
			}
			if (!isWorldNote(target.trim())) return label;
			return `<a class="stub" data-title="${escapeHtml(title)}" href="#" title="Not written yet">${escapeHtml(label)}</a>`;
		},
	);
}

/** The world notes a note links to, resolved and not. */
function linkTargets(markdown: string): string[] {
	return [...markdown.matchAll(WIKILINK)]
		.map((match) => match[1].trim())
		.filter(isWorldNote)
		.map((target) => target.replace(NOTE_PREFIX, ""));
}

// ─── Sources ─────────────────────────────────────────────────────────────────

/** Read the Primer note: each `## Group` heading followed by wikilinks. */
export function readPrimerGroups(): { group: string; titles: string[] }[] {
	const primer = readFileSync(join(VAULT_DIR, PRIMER_NOTE), "utf8");
	const groups: { group: string; titles: string[] }[] = [];
	for (const line of primer.split("\n")) {
		const heading = line.match(/^## (.+)/);
		if (heading) {
			groups.push({ group: heading[1].trim(), titles: [] });
			continue;
		}
		const link = line.match(/\[\[([^\]|#]+)/);
		const current = groups.at(-1);
		if (link && current) {
			current.titles.push(link[1].trim().replace(NOTE_PREFIX, ""));
		}
	}
	return groups;
}

/** The vault file a note of this title lives in. */
export const notePath = (title: string): string =>
	join(VAULT_DIR, `${NOTE_PREFIX}${title}.md`);

export function loadNotes(): Note[] {
	const notes: Note[] = [];
	for (const { group, titles } of readPrimerGroups()) {
		const section: Section = LORE_GROUPS.includes(group) ? "lore" : "design";
		for (const title of titles) {
			const path = notePath(title);
			if (!existsSync(path)) {
				console.warn(`skipped: no vault note for "${title}"`);
				continue;
			}
			const slug = slugify(title);
			const url =
				section === "lore" ? `/lore/${slug}/` : `/design/notes/${slug}/`;
			const markdown = stripLeadingTitle(
				stripFrontmatter(readFileSync(path, "utf8")),
			);
			notes.push({
				title,
				group,
				section,
				url,
				markdown,
				linksTo: new Set(),
				stubs: new Set(),
			});
		}
	}
	const titles = new Set(notes.map((note) => note.title));
	for (const note of notes) {
		for (const target of linkTargets(note.markdown)) {
			if (target === note.title) continue;
			(titles.has(target) ? note.linksTo : note.stubs).add(target);
		}
	}
	return notes;
}

function buildWiki(notes: Note[], specs: Spec[]): Wiki {
	const backlinks = new Map<string, string[]>();
	for (const note of notes) {
		for (const target of note.linksTo) {
			backlinks.set(target, [...(backlinks.get(target) ?? []), note.title]);
		}
	}
	return {
		notes,
		notesByTitle: new Map(notes.map((note) => [note.title, note])),
		specs,
		backlinks,
	};
}

/**
 * Specs reference files by absolute local path. Point links to other specs at
 * their pages, reduce other repo links to their label, and shorten any
 * remaining home-directory path to `~`, so no local path is published.
 */
function resolveLocalPaths(markdown: string): string {
	const withoutRepoLinks = markdown.replace(
		/\[([^\]]+)\]\((\/[^)\s]+)\)/g,
		(match, label: string, path: string) => {
			if (!path.startsWith(REPO_DIR)) return match;
			const specId = path.match(/\/specs\/([^-/]+)-[^/]*\.md$/)?.[1];
			return specId ? `[${label}](/design/specs/${specId}/)` : label;
		},
	);
	return withoutRepoLinks.replaceAll(homedir(), "~");
}

function loadSpecs(): Spec[] {
	const specsDir = join(REPO_DIR, "specs");
	return readdirSync(specsDir)
		.filter((file) => file.endsWith(".md"))
		.sort()
		.map((file) => {
			const source = readFileSync(join(specsDir, file), "utf8");
			const id = file.split("-")[0];
			const heading = source.match(/^# (.+)/m)?.[1] ?? file;
			// "Spec 004 - Agent Playtest Runtime" -> "Agent Playtest Runtime"
			const title = heading.replace(/^Spec \S+ [-—] /, "");
			return {
				id,
				title,
				url: `/design/specs/${id}/`,
				markdown: resolveLocalPaths(stripLeadingTitle(source)),
			};
		});
}

function readCommits(): Commit[] {
	const log = execFileSync(
		"git",
		["log", "--format=%ad%x09%s", "--date=short"],
		{ cwd: REPO_DIR, encoding: "utf8" },
	);
	return log
		.trim()
		.split("\n")
		.map((line) => {
			const [date, ...subject] = line.split("\t");
			return { date, subject: subject.join("\t") };
		});
}

/**
 * How commits become devlog entries. Currently: one entry per day, newest
 * first, listing every commit subject from that day.
 */
function groupCommitsIntoEntries(commits: Commit[]): DevlogEntry[] {
	const entries = new Map<string, string[]>();
	for (const { date, subject } of commits) {
		const changes = entries.get(date) ?? [];
		changes.push(subject);
		entries.set(date, changes);
	}
	return [...entries].map(([date, changes]) => ({ date, changes }));
}

// ─── Page rendering ──────────────────────────────────────────────────────────

function renderPage(page: {
	title: string;
	activeNav: string;
	body: string;
	scripts?: string[];
	/** Use the full window width instead of the reading column. */
	wide?: boolean;
	/** A sidebar beside the main column: the wiki's tree and search. */
	aside?: string;
}): string {
	const navLinks = NAV.map(({ label, url }) => {
		const current = url === page.activeNav ? ' aria-current="page"' : "";
		return `<a href="${url}"${current}>${label}</a>`;
	}).join("\n    ");
	const scripts = (page.scripts ?? [])
		.map((src) => `<script src="${src}" defer></script>`)
		.join("\n");
	const fullTitle =
		page.title === "Margin's Grip"
			? page.title
			: `${page.title} | Margin's Grip`;
	return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(fullTitle)}</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500&family=Public+Sans:wght@400;600;700&display=swap" rel="stylesheet">
<link rel="stylesheet" href="/assets/site.css">
<script>try{const t=localStorage.getItem("theme");if(t)document.documentElement.dataset.theme=t}catch{}</script>
<script src="/assets/theme.js" defer></script>
${scripts}
</head>
<body>
<div class="page${page.wide ? " page-wide" : ""}">
  <nav class="nav" aria-label="Site">
    <strong>Margin's Grip</strong>
    ${navLinks}
    <button type="button" class="plain theme-toggle" id="themeToggle">Dark</button>
  </nav>
${page.aside ? `  <div class="wiki">\n${page.aside}\n  <main>\n${page.body}\n  </main>\n  </div>` : `  <main>\n${page.body}\n  </main>`}
</div>
</body>
</html>
`;
}

function writePage(url: string, html: string): void {
	const directory = join(OUT_DIR, url);
	mkdirSync(directory, { recursive: true });
	writeFileSync(join(directory, "index.html"), html);
}

const renderContentFile = (name: string): string =>
	markdownRenderer.render(
		readFileSync(join(SITE_DIR, "content", name), "utf8"),
	);

function noteTable(notes: Note[], notesByTitle: Map<string, Note>): string {
	const rows = notes.map((note) => {
		const summary = summarise(resolveWikilinks(note.markdown, notesByTitle));
		return `<tr><th><a href="${note.url}">${escapeHtml(note.title)}</a></th><td>${escapeHtml(summary)}</td></tr>`;
	});
	return `<table>\n${rows.join("\n")}\n</table>`;
}

/** One `<h2>` and table per Primer group, in Primer order. */
function groupedNoteTables(
	notes: Note[],
	notesByTitle: Map<string, Note>,
): string {
	const groups = [...new Set(notes.map((note) => note.group))];
	return groups
		.map((group) => {
			const groupNotes = notes.filter((note) => note.group === group);
			return `<h2>${escapeHtml(group)}</h2>\n${noteTable(groupNotes, notesByTitle)}`;
		})
		.join("\n");
}

/**
 * The wiki's sidebar: a search box and every page in the tree, lore then
 * design then specs, with the current page marked.
 */
function renderWikiSidebar(wiki: Wiki, currentUrl: string): string {
	const link = (url: string, label: string) =>
		`<li><a href="${url}"${url === currentUrl ? ' aria-current="page"' : ""}>${escapeHtml(label)}</a></li>`;
	const section = (label: string, url: string, notes: Note[]) => {
		const groups = [...new Set(notes.map((note) => note.group))];
		const lists = groups
			.map((group) => {
				const items = notes
					.filter((note) => note.group === group)
					.map((note) => link(note.url, note.title))
					.join("\n");
				return `<li class="wiki-group">${escapeHtml(group)}<ul>\n${items}\n</ul></li>`;
			})
			.join("\n");
		return `<li class="wiki-section"><a href="${url}"${url === currentUrl ? ' aria-current="page"' : ""}>${label}</a><ul>\n${lists}\n</ul></li>`;
	};
	const specItems = wiki.specs
		.map((spec) => link(spec.url, `${spec.id} ${spec.title}`))
		.join("\n");
	return `  <aside class="wiki-nav" aria-label="Wiki">
<input type="search" id="wikiSearch" placeholder="Search" aria-label="Search the wiki" autocomplete="off">
<ol id="wikiResults" class="wiki-results" hidden></ol>
<ul class="wiki-tree">
${section(
	"Lore",
	"/lore/",
	wiki.notes.filter((note) => note.section === "lore"),
)}
${section(
	"Design",
	"/design/",
	wiki.notes.filter((note) => note.section === "design"),
)}
<li class="wiki-section"><a href="/design/#specs">Specs</a><ul>\n${specItems}\n</ul></li>
</ul>
  </aside>`;
}

/** A contents list from the page's second- and third-level headings. */
function renderContents(markdown: string): string {
	const headings = [...markdown.matchAll(/^(##|###) (.+)$/gm)].map(
		([, level, text]) => ({
			level: level.length,
			text: text.replace(WIKILINK, (_m, target, heading, alias) =>
				(alias ?? heading ?? target.replace(NOTE_PREFIX, "")).trim(),
			),
		}),
	);
	if (headings.length < 3) return "";
	// A long contents list is worse than none: past this many entries, only
	// the second-level headings are listed.
	const shown =
		headings.length > CONTENTS_MAX_ENTRIES
			? headings.filter((heading) => heading.level === 2)
			: headings;
	const items = shown
		.map(
			({ level, text }) =>
				`<li class="toc-${level}"><a href="#${slugify(text)}">${escapeHtml(text)}</a></li>`,
		)
		.join("\n");
	return `<nav class="toc" aria-label="Contents"><strong>Contents</strong><ol>\n${items}\n</ol></nav>`;
}

function renderBacklinks(wiki: Wiki, title: string): string {
	const from = wiki.backlinks.get(title) ?? [];
	if (from.length === 0) return "";
	const items = from
		.map((other) => wiki.notesByTitle.get(other))
		.filter((note): note is Note => note !== undefined)
		.map(
			(note) => `<li><a href="${note.url}">${escapeHtml(note.title)}</a></li>`,
		)
		.join("\n");
	return `<section class="backlinks"><h2>Linked from</h2><ul>\n${items}\n</ul></section>`;
}

function renderNotePage(note: Note, wiki: Wiki): string {
	const sectionUrl = note.section === "lore" ? "/lore/" : "/design/";
	const sectionLabel = note.section === "lore" ? "Lore" : "Design";
	const prototypeNotice = PROTOTYPE_ERA_GROUPS.includes(note.group)
		? '<p class="notice">Written for the earlier Bevy prototype. Kept for reference; the repository is the current source.</p>'
		: "";
	const body = `<p class="crumb"><a href="${sectionUrl}">${sectionLabel}</a> / ${escapeHtml(note.group)}</p>
<article class="prose" data-note="${escapeHtml(note.title)}">
<h1>${escapeHtml(note.title)}</h1>
${prototypeNotice}
${renderContents(note.markdown)}
${markdownRenderer.render(resolveWikilinks(note.markdown, wiki.notesByTitle))}
</article>
${renderBacklinks(wiki, note.title)}`;
	return renderPage({
		title: note.title,
		activeNav: sectionUrl,
		body,
		aside: renderWikiSidebar(wiki, note.url),
		scripts: ["/assets/wiki.js"],
	});
}

function renderSpecPage(spec: Spec, wiki: Wiki): string {
	const body = `<p class="crumb"><a href="/design/">Design</a> / Specs</p>
<article class="prose">
<h1>${escapeHtml(spec.title)}</h1>
${renderContents(spec.markdown)}
${markdownRenderer.render(spec.markdown)}
</article>`;
	return renderPage({
		title: `Spec ${spec.id}: ${spec.title}`,
		activeNav: "/design/",
		body,
		aside: renderWikiSidebar(wiki, spec.url),
		scripts: ["/assets/wiki.js"],
	});
}

/** Plain text of a note, for the search index. */
function plainText(markdown: string): string {
	return markdown
		.replace(WIKILINK, (_m, target, heading, alias) =>
			(alias ?? heading ?? target.replace(NOTE_PREFIX, "")).trim(),
		)
		.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
		.replace(/[#*_`>|-]+/g, " ")
		.replace(/\s+/g, " ")
		.trim();
}

function writeSearchIndex(wiki: Wiki): void {
	const entries = [
		...wiki.notes.map((note) => ({
			title: note.title,
			url: note.url,
			where: `${note.section === "lore" ? "Lore" : "Design"} / ${note.group}`,
			text: plainText(note.markdown),
		})),
		...wiki.specs.map((spec) => ({
			title: `${spec.id} ${spec.title}`,
			url: spec.url,
			where: "Specs",
			text: plainText(spec.markdown),
		})),
	];
	writeFileSync(join(OUT_DIR, "wiki-index.json"), JSON.stringify(entries));
}

function renderDevlog(entries: DevlogEntry[]): string {
	const sections = entries.map(({ date, changes }) => {
		const items = changes
			.map((change) => `<li>${escapeHtml(change)}</li>`)
			.join("\n");
		return `<h2><time datetime="${date}">${date}</time></h2>\n<ul>\n${items}\n</ul>`;
	});
	return `<h1>Devlog</h1>\n<p>Commit history, grouped by day.</p>\n${sections.join("\n")}`;
}

// ─── Build ───────────────────────────────────────────────────────────────────

/** Bundle the wiki editor (CodeMirror with vim) into one browser script. */
async function bundleEditor(): Promise<void> {
	const result = await Bun.build({
		entrypoints: [join(SITE_DIR, "assets", "editor.ts")],
		outdir: join(OUT_DIR, "assets"),
		minify: true,
		format: "esm",
	});
	if (!result.success) {
		for (const log of result.logs) console.error(log);
		throw new Error("bundling the editor failed");
	}
}

export async function build(): Promise<void> {
	mkdirSync(OUT_DIR, { recursive: true });
	cpSync(join(SITE_DIR, "assets"), join(OUT_DIR, "assets"), {
		recursive: true,
		// Bun's cpSync stops overwriting once a filter is given unless told to.
		force: true,
		// Sources of the bundle stay out of the output; the bundle is written next.
		filter: (source) => !source.endsWith(".ts"),
	});
	await bundleEditor();
	// The terrain generator for the map's zoomed-in tiles, if it has been built.
	if (existsSync(TERRAIN_WASM)) {
		cpSync(TERRAIN_WASM, join(OUT_DIR, "assets", "terrain.wasm"));
	} else {
		console.warn(
			"terrain.wasm not built: the map will not sharpen on zoom. Build it with: cargo build -p mg_web --release --target wasm32-unknown-unknown (in gdextension/)",
		);
	}

	const notes = loadNotes();
	const specs = loadSpecs();
	const wiki = buildWiki(notes, specs);
	const { notesByTitle } = wiki;
	const loreNotes = notes.filter((note) => note.section === "lore");
	const designNotes = notes.filter((note) => note.section === "design");

	writePage(
		"/",
		renderPage({
			title: "Margin's Grip",
			activeNav: "/",
			body: renderContentFile("overview.md"),
		}),
	);

	writePage(
		"/map/",
		renderPage({
			title: "Map",
			activeNav: "/map/",
			body: readFileSync(join(SITE_DIR, "content", "map.html"), "utf8"),
			scripts: ["/assets/map.js"],
			wide: true,
		}),
	);

	writePage(
		"/sandbox/",
		renderPage({
			title: "Erosion sandbox",
			activeNav: "/design/",
			body: readFileSync(join(SITE_DIR, "content", "sandbox.html"), "utf8"),
			scripts: ["/assets/sandbox.js"],
			wide: true,
		}),
	);

	writePage(
		"/lore/",
		renderPage({
			title: "Lore",
			activeNav: "/lore/",
			body: `<h1>Lore</h1>\n${groupedNoteTables(loreNotes, notesByTitle)}`,
			aside: renderWikiSidebar(wiki, "/lore/"),
			scripts: ["/assets/wiki.js"],
		}),
	);

	const specRows = specs
		.map(
			(spec) =>
				`<tr><td class="num">${spec.id}</td><td><a href="${spec.url}">${escapeHtml(spec.title)}</a></td></tr>`,
		)
		.join("\n");
	writePage(
		"/design/",
		renderPage({
			title: "Design",
			activeNav: "/design/",
			body: `${renderContentFile("design.md")}
${groupedNoteTables(designNotes, notesByTitle)}
<h2 id="specs">Specs</h2>
<table>\n${specRows}\n</table>`,
			aside: renderWikiSidebar(wiki, "/design/"),
			scripts: ["/assets/wiki.js"],
		}),
	);

	for (const note of notes) {
		writePage(note.url, renderNotePage(note, wiki));
	}
	for (const spec of specs) {
		writePage(spec.url, renderSpecPage(spec, wiki));
	}
	writeSearchIndex(wiki);

	const entries = groupCommitsIntoEntries(readCommits());
	writePage(
		"/devlog/",
		renderPage({
			title: "Devlog",
			activeNav: "/devlog/",
			body: renderDevlog(entries),
		}),
	);

	console.log(
		`built ${notes.length} notes, ${specs.length} specs, ${entries.length} devlog days into ${OUT_DIR}`,
	);
}

if (import.meta.main) await build();
