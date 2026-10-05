// Renders terrain tiles for the map page, off the main thread.
//
// The terrain generator is the game's own, compiled to WebAssembly
// (gdextension/crates/mg_web), reading the same macro pack the game loads.
// Its interface is a few plain functions over the module's memory.

let terrain = null;

async function load(wasmUrl, packUrl) {
	const [wasm, pack] = await Promise.all(
		[wasmUrl, packUrl].map(async (url) => {
			const response = await fetch(url);
			if (!response.ok) throw new Error(`${url}: ${response.status}`);
			return new Uint8Array(await response.arrayBuffer());
		}),
	);
	const { instance } = await WebAssembly.instantiate(wasm, {});
	const pointer = instance.exports.mg_alloc(pack.length);
	new Uint8Array(instance.exports.memory.buffer, pointer, pack.length).set(
		pack,
	);
	const loaded = instance.exports.mg_load_pack(pointer, pack.length);
	instance.exports.mg_free(pointer, pack.length);
	if (!loaded) throw new Error(`${packUrl} is not a readable macro pack`);
	terrain = instance.exports;
}

// RGBA pixels of a square tile: `span` chunks from (x, y), `pixels` on a side.
function renderTile({ x, y, span, pixels }) {
	const pointer = terrain.mg_render_tile(x, y, span, pixels);
	// Copy out: the module reuses this memory for the next tile.
	return new Uint8Array(
		terrain.memory.buffer,
		pointer,
		pixels * pixels * 4,
	).slice();
}

self.addEventListener("message", async ({ data }) => {
	if (data.type === "load") {
		try {
			await load(data.wasmUrl, data.packUrl);
			self.postMessage({ type: "ready" });
		} catch (error) {
			self.postMessage({ type: "failed", reason: String(error) });
		}
		return;
	}
	const pixels = renderTile(data);
	self.postMessage({ type: "tile", key: data.key, pixels }, [pixels.buffer]);
});
