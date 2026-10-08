// The wiki's sidebar search. The index (title, where, text of every page)
// is built with the site; matching is a plain substring search over it,
// titles first, with a snippet around the first hit in the text.
const input = document.getElementById("wikiSearch");
const results = document.getElementById("wikiResults");
const tree = document.querySelector(".wiki-tree");
const MAX_RESULTS = 12;
const SNIPPET = 70;

let index = null;
async function loadIndex() {
	if (index) return index;
	const response = await fetch("/wiki-index.json");
	index = await response.json();
	return index;
}

function snippet(text, query) {
	const at = text.toLowerCase().indexOf(query);
	if (at < 0) return "";
	const start = Math.max(0, at - SNIPPET / 2);
	const end = Math.min(text.length, at + query.length + SNIPPET / 2);
	return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
}

async function search(query) {
	const q = query.trim().toLowerCase();
	if (q.length < 2) {
		results.hidden = true;
		tree.hidden = false;
		return;
	}
	const pages = await loadIndex();
	const inTitle = pages.filter((page) => page.title.toLowerCase().includes(q));
	const inText = pages.filter(
		(page) => !inTitle.includes(page) && page.text.toLowerCase().includes(q),
	);
	const hits = [...inTitle, ...inText].slice(0, MAX_RESULTS);
	results.replaceChildren(
		...hits.map((page) => {
			const item = document.createElement("li");
			const link = document.createElement("a");
			link.href = page.url;
			link.textContent = page.title;
			const where = document.createElement("small");
			where.textContent = page.where;
			item.append(link, where);
			const found = snippet(page.text, q);
			if (found) {
				const text = document.createElement("p");
				text.textContent = found;
				item.append(text);
			}
			return item;
		}),
	);
	if (hits.length === 0) {
		const none = document.createElement("li");
		none.className = "muted";
		none.textContent = "Nothing found";
		results.append(none);
	}
	results.hidden = false;
	tree.hidden = true;
}

input.addEventListener("input", () => search(input.value));
input.addEventListener("keydown", (event) => {
	if (event.key === "Escape") {
		input.value = "";
		search("");
	}
});
// "/" focuses the search from anywhere on the page, as on most wikis.
document.addEventListener("keydown", (event) => {
	if (event.key === "/" && document.activeElement !== input) {
		event.preventDefault();
		input.focus();
	}
});

// ── Editing ──────────────────────────────────────────────────────────────────
// Only the local server has the API; anywhere else these controls never
// appear and the wiki is read-only.

const article = document.querySelector("article[data-note]");
const slug = location.pathname.split("/").filter(Boolean).at(-1);

async function api(path, options) {
	const response = await fetch(path, {
		headers: { "Content-Type": "application/json" },
		...options,
	});
	if (!response.ok) {
		const body = await response.json().catch(() => ({}));
		throw new Error(body.error ?? response.statusText);
	}
	return response.json();
}

function button(label, onClick) {
	const element = document.createElement("button");
	element.type = "button";
	element.className = "plain small";
	element.textContent = label;
	element.addEventListener("click", onClick);
	return element;
}

/**
 * Swap the page's text for the editor (CodeMirror with vim keys; jk leaves
 * insert mode, :w saves, :q closes). Save writes the vault note.
 */
async function openEditor() {
	const [note, { openEditor: open }] = await Promise.all([
		api(`/api/notes/${slug}`),
		import("/assets/editor.js"),
	]);
	const editor = document.createElement("div");
	editor.className = "editor";
	const controls = document.createElement("p");
	controls.className = "editor-controls";
	const status = document.createElement("span");
	status.className = "muted";
	const pane = document.createElement("div");
	pane.className = "editor-pane";
	editor.append(controls, pane);
	article.replaceWith(editor);
	let text = () => note.markdown;
	const save = async () => {
		status.textContent = "Saving…";
		try {
			await api(`/api/notes/${slug}`, {
				method: "PUT",
				body: JSON.stringify({ markdown: text() }),
			});
			location.reload();
		} catch (error) {
			status.textContent = `Not saved: ${error.message}`;
		}
	};
	const close = () => location.reload();
	const hint = document.createElement("span");
	hint.className = "muted";
	hint.textContent = "vim keys: jk to leave insert, :w saves, :q closes";
	controls.append(button("Save", save), button("Cancel", close), hint, status);
	const opened = open(pane, note.markdown, { save, close });
	text = opened.text;
	editor.addEventListener("keydown", (event) => {
		if ((event.metaKey || event.ctrlKey) && event.key === "s") {
			event.preventDefault();
			save();
		}
	});
}

/** Ask for a group and make the page, then open it. */
async function createPage(title) {
	const { groups } = await api("/api/notes");
	const name = title ?? prompt("Title of the new page");
	if (!name) return;
	const group = prompt(`Which group? One of:\n${groups.join(", ")}`, groups[0]);
	if (!group) return;
	try {
		const { url } = await api("/api/notes", {
			method: "POST",
			body: JSON.stringify({ title: name, group }),
		});
		location.href = `${url}#edit`;
	} catch (error) {
		alert(`Not created: ${error.message}`);
	}
}

async function enableEditing() {
	try {
		await api("/api/notes");
	} catch {
		return;
	}
	const tools = document.createElement("p");
	tools.className = "wiki-tools";
	tools.append(button("New page", () => createPage()));
	if (article) {
		tools.append(button("Edit", openEditor));
	}
	tree.before(tools);
	for (const stub of document.querySelectorAll("a.stub")) {
		stub.title = "Not written yet: click to create";
		stub.addEventListener("click", (event) => {
			event.preventDefault();
			createPage(stub.dataset.title);
		});
	}
	if (article && location.hash === "#edit") openEditor();
}
enableEditing();
