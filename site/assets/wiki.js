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
