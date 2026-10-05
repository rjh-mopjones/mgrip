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
const VAULT_DIR =
	process.env.MG_VAULT_DIR ??
	join(homedir(), "Documents/mop-jones-brain/Notes");

const NOTE_PREFIX = "Margin's Grip - ";
const PRIMER_NOTE = "Margin's Grip Game World Primer.md";
/** Primer groups shown under Lore. Every other group goes under Design. */
const LORE_GROUPS = ["World", "Quests"];
/** Primer groups written for the earlier Bevy prototype. */
const PROTOTYPE_ERA_GROUPS = ["Technical"];
const SUMMARY_MAX_LENGTH = 160;

const NAV = [
	{ label: "Overview", url: "/" },
	{ label: "Map", url: "/map/" },
	{ label: "Lore", url: "/lore/" },
	{ label: "Design", url: "/design/" },
	{ label: "Devlog", url: "/devlog/" },
];

type Section = "lore" | "design";

interface Note {
	title: string;
	group: string;
	section: Section;
	url: string;
	markdown: string;
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

const slugify = (text: string): string =>
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

const stripFrontmatter = (markdown: string): string =>
	markdown.replace(/^---\n[\s\S]*?\n---\n/, "");

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

/**
 * Turn Obsidian wikilinks into markdown links. Links to notes that are not
 * published (journal entries, categories) become plain text.
 */
function resolveWikilinks(
	markdown: string,
	notesByTitle: Map<string, Note>,
): string {
	return markdown.replace(
		/!?\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g,
		(_match, target: string, heading?: string, alias?: string) => {
			const title = target.trim().replace(NOTE_PREFIX, "");
			const label = alias ?? heading ?? title;
			const note = notesByTitle.get(title);
			if (!note) return label;
			const anchor = heading ? `#${slugify(heading)}` : "";
			return `[${label}](${note.url}${anchor})`;
		},
	);
}

// ─── Sources ─────────────────────────────────────────────────────────────────

/** Read the Primer note: each `## Group` heading followed by wikilinks. */
function readPrimerGroups(): { group: string; titles: string[] }[] {
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

function loadNotes(): Note[] {
	const notes: Note[] = [];
	for (const { group, titles } of readPrimerGroups()) {
		const section: Section = LORE_GROUPS.includes(group) ? "lore" : "design";
		for (const title of titles) {
			const path = join(VAULT_DIR, `${NOTE_PREFIX}${title}.md`);
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
			notes.push({ title, group, section, url, markdown });
		}
	}
	return notes;
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
${scripts}
</head>
<body>
<div class="page${page.wide ? " page-wide" : ""}">
  <nav class="nav" aria-label="Site">
    <strong>Margin's Grip</strong>
    ${navLinks}
  </nav>
  <main>
${page.body}
  </main>
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

function renderNotePage(note: Note, notesByTitle: Map<string, Note>): string {
	const sectionUrl = note.section === "lore" ? "/lore/" : "/design/";
	const sectionLabel = note.section === "lore" ? "Lore" : "Design";
	const prototypeNotice = PROTOTYPE_ERA_GROUPS.includes(note.group)
		? '<p class="notice">Written for the earlier Bevy prototype. Kept for reference; the repository is the current source.</p>'
		: "";
	const body = `<p class="crumb"><a href="${sectionUrl}">${sectionLabel}</a> / ${escapeHtml(note.group)}</p>
<article class="prose">
<h1>${escapeHtml(note.title)}</h1>
${prototypeNotice}
${markdownRenderer.render(resolveWikilinks(note.markdown, notesByTitle))}
</article>`;
	return renderPage({ title: note.title, activeNav: sectionUrl, body });
}

function renderSpecPage(spec: Spec): string {
	const body = `<p class="crumb"><a href="/design/">Design</a> / Specs</p>
<article class="prose">
<h1>${escapeHtml(spec.title)}</h1>
${markdownRenderer.render(spec.markdown)}
</article>`;
	return renderPage({
		title: `Spec ${spec.id}: ${spec.title}`,
		activeNav: "/design/",
		body,
	});
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

function build(): void {
	mkdirSync(OUT_DIR, { recursive: true });
	cpSync(join(SITE_DIR, "assets"), join(OUT_DIR, "assets"), {
		recursive: true,
	});
	// The terrain generator for the map's zoomed-in tiles, if it has been built.
	if (existsSync(TERRAIN_WASM)) {
		cpSync(TERRAIN_WASM, join(OUT_DIR, "assets", "terrain.wasm"));
	} else {
		console.warn(
			"terrain.wasm not built: the map will not sharpen on zoom. Build it with: cargo build -p mg_web --release --target wasm32-unknown-unknown (in gdextension/)",
		);
	}

	const notes = loadNotes();
	const notesByTitle = new Map(notes.map((note) => [note.title, note]));
	const loreNotes = notes.filter((note) => note.section === "lore");
	const designNotes = notes.filter((note) => note.section === "design");
	const specs = loadSpecs();

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
		"/lore/",
		renderPage({
			title: "Lore",
			activeNav: "/lore/",
			body: `<h1>Lore</h1>\n${groupedNoteTables(loreNotes, notesByTitle)}`,
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
<h2>Specs</h2>
<table>\n${specRows}\n</table>`,
		}),
	);

	for (const note of notes) {
		writePage(note.url, renderNotePage(note, notesByTitle));
	}
	for (const spec of specs) {
		writePage(spec.url, renderSpecPage(spec));
	}

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

build();
