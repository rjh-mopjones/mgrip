// Runs the erosion sandbox off the main thread: land grown from uplift and
// cut by rivers on Margin's real coastline (gdextension/crates/mg_web,
// sandbox.rs). The page sends settings; this steps, draws and reports.

importScripts("/assets/terrain-module.js");

let terrain = null;
let width = 0;

// The order mg_sandbox_step takes its settings in.
const stepArguments = (settings) => [
	settings.erodibility,
	settings.uplift,
	settings.flowExponent,
	settings.slopeCreep,
	settings.upliftLimit,
	settings.nightRunoff,
	settings.dayRunoff,
	settings.interiorUplift,
	settings.rangeUplift,
	settings.faultUplift,
	settings.iceWidening,
];

function draw(settings) {
	const pointer = terrain.mg_sandbox_render(
		settings.view,
		settings.riverThreshold,
		settings.nightRunoff,
		settings.dayRunoff,
		settings.interiorUplift,
		settings.rangeUplift,
		settings.faultUplift,
	);
	const pixels = new Uint8ClampedArray(
		terrain.memory.buffer,
		pointer,
		width * (width / 2) * 4,
	).slice();
	const stat = (which) => terrain.mg_sandbox_stat(which);
	self.postMessage(
		{
			type: "frame",
			width,
			pixels,
			steps: stat(0),
			peakBlocks: stat(1),
			riverCells: stat(2),
			lakeCells: stat(3),
			changeBlocks: stat(4),
		},
		[pixels.buffer],
	);
}

self.addEventListener("message", async ({ data }) => {
	if (data.type === "load") {
		try {
			terrain = await loadTerrain(data.wasmUrl, data.packUrl);
			width = terrain.mg_sandbox_reset();
			self.postMessage({ type: "ready" });
		} catch (error) {
			self.postMessage({ type: "failed", reason: String(error) });
		}
		return;
	}
	if (data.type === "reset") terrain.mg_sandbox_reset();
	if (data.steps > 0) {
		terrain.mg_sandbox_step(data.steps, ...stepArguments(data.settings));
	}
	draw(data.settings);
});
