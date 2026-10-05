// Renders terrain tiles for the map page, off the main thread, with the
// game's own terrain generator (see terrain-module.js).

importScripts("/assets/terrain-module.js");

let terrain = null;

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
			terrain = await loadTerrain(data.wasmUrl, data.packUrl);
			self.postMessage({ type: "ready" });
		} catch (error) {
			self.postMessage({ type: "failed", reason: String(error) });
		}
		return;
	}
	const pixels = renderTile(data);
	self.postMessage({ type: "tile", key: data.key, pixels }, [pixels.buffer]);
});
