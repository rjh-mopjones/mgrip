// Loads the terrain generator (gdextension/crates/mg_web, compiled to
// WebAssembly) and gives it the macro pack. Shared by the workers that use
// it; load with importScripts.
//
// The module's interface is a few plain functions over its own memory.

// Returns the module's exports once the pack at `packUrl` is loaded into it.
self.loadTerrain = async (wasmUrl, packUrl) => {
	const [wasm, pack] = await Promise.all(
		[wasmUrl, packUrl].map(async (url) => {
			const response = await fetch(url);
			if (!response.ok) throw new Error(`${url}: ${response.status}`);
			return new Uint8Array(await response.arrayBuffer());
		}),
	);
	const { instance } = await WebAssembly.instantiate(wasm, {});
	const terrain = instance.exports;
	const pointer = terrain.mg_alloc(pack.length);
	new Uint8Array(terrain.memory.buffer, pointer, pack.length).set(pack);
	const loaded = terrain.mg_load_pack(pointer, pack.length);
	terrain.mg_free(pointer, pack.length);
	if (!loaded) throw new Error(`${packUrl} is not a readable macro pack`);
	return terrain;
};
