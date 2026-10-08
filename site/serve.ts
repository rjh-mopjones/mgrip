// Serves the built site locally, and lets the wiki be edited in the browser.
//
// Static files come from site/dist with the headers the threaded Godot web
// build needs (COOP + COEP, for SharedArrayBuffer). The API under /api/
// reads and writes the Obsidian vault's notes, so the site and Obsidian edit
// the same files; every save rebuilds the site. It binds to localhost only:
// a public deploy serves the same pages with no API, and no Edit button.
//
// Usage: bun run serve [port]

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
	build,
	frontmatterOf,
	loadNotes,
	NOTE_PREFIX,
	notePath,
	PRIMER_NOTE,
	readPrimerGroups,
	slugify,
	VAULT_DIR,
} from "./build.ts";

const DIST_DIR = join(import.meta.dir, "dist");
const PORT = Number(process.argv[2] ?? 8060);
const HEADERS = {
	"Cross-Origin-Opener-Policy": "same-origin",
	"Cross-Origin-Embedder-Policy": "require-corp",
	"Cache-Control": "no-store",
};

const json = (body: unknown, status = 200) =>
	new Response(JSON.stringify(body), {
		status,
		headers: { ...HEADERS, "Content-Type": "application/json" },
	});

const noteBySlug = (slug: string) =>
	loadNotes().find((note) => slugify(note.title) === slug);

/** A note's text as the editor sees it: everything after the frontmatter. */
function readNote(slug: string): Response {
	const note = noteBySlug(slug);
	if (!note) return json({ error: "no such note" }, 404);
	const source = readFileSync(notePath(note.title), "utf8");
	return json({
		title: note.title,
		url: note.url,
		markdown: source.slice(frontmatterOf(source).length),
	});
}

/** Write the editor's text back under the note's frontmatter and rebuild. */
async function writeNote(slug: string, request: Request): Promise<Response> {
	const note = noteBySlug(slug);
	if (!note) return json({ error: "no such note" }, 404);
	const { markdown } = (await request.json()) as { markdown?: string };
	if (typeof markdown !== "string") return json({ error: "no markdown" }, 400);
	const path = notePath(note.title);
	const source = readFileSync(path, "utf8");
	writeFileSync(path, frontmatterOf(source) + markdown);
	build();
	return json({ ok: true, url: note.url });
}

/**
 * Make a new note: a file in the vault with the title as its heading, and a
 * link to it in the Primer under the group chosen, which is what publishes
 * it. Rebuilds, and answers with the new page's address.
 */
async function createNote(request: Request): Promise<Response> {
	const { title, group } = (await request.json()) as {
		title?: string;
		group?: string;
	};
	const cleanTitle = title?.trim().replace(/[/\\:]/g, "");
	if (!cleanTitle) return json({ error: "no title" }, 400);
	const groups = readPrimerGroups().map((entry) => entry.group);
	if (!group || !groups.includes(group)) {
		return json({ error: `group must be one of: ${groups.join(", ")}` }, 400);
	}
	const path = notePath(cleanTitle);
	if (!existsSync(path)) {
		writeFileSync(path, `# ${cleanTitle}\n\n`);
	}
	const primerPath = join(VAULT_DIR, PRIMER_NOTE);
	const primer = readFileSync(primerPath, "utf8");
	const link = `- [[${NOTE_PREFIX}${cleanTitle}]]`;
	if (!primer.includes(`[[${NOTE_PREFIX}${cleanTitle}]]`)) {
		// Add the link at the end of the group's list: after its heading and
		// the lines that follow it up to the next heading or blank line.
		const lines = primer.split("\n");
		const heading = lines.findIndex((line) => line.trim() === `## ${group}`);
		let end = heading + 1;
		while (end < lines.length && lines[end].startsWith("- ")) end++;
		lines.splice(end, 0, link);
		writeFileSync(primerPath, lines.join("\n"));
	}
	build();
	const note = noteBySlug(slugify(cleanTitle));
	return json({ ok: true, url: note?.url ?? "/" });
}

function serveFile(pathname: string): Response {
	let path = join(DIST_DIR, decodeURIComponent(pathname));
	if (!path.startsWith(DIST_DIR)) return new Response("Forbidden", { status: 403 });
	if (pathname.endsWith("/")) path = join(path, "index.html");
	if (!existsSync(path)) {
		return new Response("File not found", { status: 404, headers: HEADERS });
	}
	return new Response(Bun.file(path), { headers: HEADERS });
}

Bun.serve({
	port: PORT,
	hostname: "localhost",
	async fetch(request) {
		const { pathname } = new URL(request.url);
		const note = pathname.match(/^\/api\/notes\/([^/]+)$/)?.[1];
		if (pathname === "/api/notes" && request.method === "GET") {
			return json({ groups: readPrimerGroups().map((entry) => entry.group) });
		}
		if (pathname === "/api/notes" && request.method === "POST") {
			return createNote(request);
		}
		if (note && request.method === "GET") return readNote(note);
		if (note && request.method === "PUT") return writeNote(note, request);
		if (pathname.startsWith("/api/")) return json({ error: "not found" }, 404);
		return serveFile(pathname);
	},
});
console.log(`Serving ${DIST_DIR} at http://localhost:${PORT} (editing on)`);
