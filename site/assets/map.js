// Map page: a pannable, zoomable view of the world. Pick a spawn chunk and
// launch the web build there.
//
// Map data is exported by the Rust CLI (`margins_grip export site-map`) into
// this page's directory: one PNG per base layer, transparent overlay PNGs,
// chunks.bin with one record per chunk, and map.json describing them all.

const DEFAULT_SPAWN_CHUNK = { x: 440, y: 220 };
const DEFAULT_LAYER_STEM = "macromap";
const MAX_ZOOM = 12;
const ZOOM_STEP = 1.5;
/// A press that moves further than this many pixels is a drag, not a click.
const DRAG_THRESHOLD_PX = 4;
/// Overlays that show faction territory get the names of the authored states.
const LABELLED_OVERLAYS = ["Factions"];

const spawn = { ...DEFAULT_SPAWN_CHUNK };
let worldMap = null;
let baseLayer = `${DEFAULT_LAYER_STEM}.png`;
const activeOverlays = new Set();
const images = new Map();
// Centre of the view in chunk coordinates; zoom 1 fits the whole world.
const view = { x: 512, y: 256, zoom: 1 };

const canvas = document.getElementById("mapCanvas");
const context = canvas.getContext("2d");
const mapFrame = document.getElementById("mapFrame");

// 'InnerTerminus' -> 'Inner terminus'
const readable = (name) =>
	name
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.replace(/ ([A-Z])/g, (_match, letter) => ` ${letter.toLowerCase()}`);

// 'light_level.png' -> 'Light level', 'lifegen_habitability.png' -> 'Habitability';
// the macromap is the default terrain view.
function layerLabel(fileName) {
	const stem = fileName.replace(/\.png$/, "");
	if (stem === DEFAULT_LAYER_STEM) return "Terrain";
	const words = stem
		.replace(/^lifegen_/, "")
		.replace(/_/g, " ")
		.toLowerCase();
	return words.charAt(0).toUpperCase() + words.slice(1);
}

// ─── Chunk data ──────────────────────────────────────────────────────────────

function chunkAt(x, y) {
	const { meta, chunks } = worldMap;
	const offset = (y * meta.chunks_wide + x) * meta.chunk_fields.length;
	return {
		light: chunks[offset] / 255,
		zone: meta.zones[chunks[offset + 1]],
		biome: meta.biomes[chunks[offset + 2]],
		// 16-bit id, low byte first; 0 = no province (ocean).
		provinceId: chunks[offset + 3] | (chunks[offset + 4] << 8),
	};
}

// "412: 108 chunks, steppe, habitability 0.61, coastal"
function provinceText(provinceId) {
	const province = worldMap.meta.provinces?.[provinceId - 1];
	if (!province) return "none";
	const facts = [
		`${province.area_chunks} chunks`,
		readable(province.biome).toLowerCase(),
		`habitability ${province.habitability.toFixed(2)}`,
	];
	if (province.coastal) facts.push("coastal");
	if (province.major_river) facts.push("major river");
	return `${provinceId}: ${facts.join(", ")}`;
}

// "Corazon (capital Violetta), 21 provinces", "Minor state 34, 9 provinces",
// or "unclaimed" / "uninhabited" for land no faction holds.
function factionText(provinceId) {
	const province = worldMap.meta.provinces?.[provinceId - 1];
	if (!province) return "none";
	const faction = worldMap.meta.factions?.[province.faction - 1];
	if (!faction) return province.state;
	const name = faction.name ?? `Minor state ${province.faction}`;
	const capital = faction.capital_name
		? ` (capital ${faction.capital_name})`
		: "";
	return `${name}${capital}, ${faction.provinces} provinces`;
}

// Chunks between two columns, the short way round the map.
function columnDistance(a, b) {
	const direct = Math.abs(a - b);
	return Math.min(direct, worldMap.meta.chunks_wide - direct);
}

// "City, 3 chunks away": the settlement closest to the spawn chunk.
function nearestSettlementText() {
	const { settlements, settlement_sizes: sizes } = worldMap.meta;
	if (!settlements?.length) return "none";
	let nearest = null;
	let nearestDistance = Number.POSITIVE_INFINITY;
	for (const settlement of settlements) {
		const distance = Math.hypot(
			columnDistance(settlement[0], spawn.x),
			settlement[1] - spawn.y,
		);
		if (distance < nearestDistance) {
			nearest = settlement;
			nearestDistance = distance;
		}
	}
	const chunks = Math.round(nearestDistance);
	const where = chunks === 0 ? "here" : `${chunks} chunks away`;
	return `${sizes[nearest[2]]}, ${where}`;
}

function renderSpawnReadout() {
	const chunk = chunkAt(spawn.x, spawn.y);
	const set = (id, text) => {
		document.getElementById(id).textContent = text;
	};
	set("spawnZone", readable(chunk.zone));
	set("spawnChunk", `${spawn.x}, ${spawn.y}`);
	set("spawnLight", chunk.light.toFixed(2));
	set("spawnBiome", readable(chunk.biome));
	set("spawnProvince", provinceText(chunk.provinceId));
	set("spawnFaction", factionText(chunk.provinceId));
	set("spawnSettlement", nearestSettlementText());
	set("spawnSeed", worldMap.meta.seed);
}

function moveSpawn(x, y) {
	const { chunks_wide: wide, chunks_high: high } = worldMap.meta;
	spawn.x = ((Math.floor(x) % wide) + wide) % wide;
	spawn.y = Math.max(0, Math.min(high - 1, Math.floor(y)));
	renderSpawnReadout();
	draw();
}

// ─── View ────────────────────────────────────────────────────────────────────

// Canvas pixels per chunk at the current zoom.
const pixelsPerChunk = () =>
	(canvas.width / worldMap.meta.chunks_wide) * view.zoom;

// Keep the view inside the map north to south, and wrap it east to west.
function constrainView() {
	const { chunks_wide: wide, chunks_high: high } = worldMap.meta;
	view.zoom = Math.max(1, Math.min(MAX_ZOOM, view.zoom));
	const halfHeight = canvas.height / pixelsPerChunk() / 2;
	view.y = Math.max(halfHeight, Math.min(high - halfHeight, view.y));
	view.x = ((view.x % wide) + wide) % wide;
}

// Chunk coordinates under a point of the canvas, given in CSS pixels.
function chunkUnder(cssX, cssY) {
	const scale = canvas.width / canvas.clientWidth;
	return {
		x: view.x + (cssX * scale - canvas.width / 2) / pixelsPerChunk(),
		y: view.y + (cssY * scale - canvas.height / 2) / pixelsPerChunk(),
	};
}

// Zoom by `factor`, keeping the chunk under (cssX, cssY) where it is.
function zoomAt(factor, cssX, cssY) {
	const before = chunkUnder(cssX, cssY);
	view.zoom *= factor;
	constrainView();
	const after = chunkUnder(cssX, cssY);
	view.x += before.x - after.x;
	view.y += before.y - after.y;
	constrainView();
	draw();
}

function resizeCanvas() {
	const width = Math.round(mapFrame.clientWidth * window.devicePixelRatio);
	canvas.width = width;
	canvas.height = width / 2;
	if (worldMap) {
		constrainView();
		draw();
	}
}

// ─── Drawing ─────────────────────────────────────────────────────────────────

function image(fileName) {
	if (!images.has(fileName)) {
		const element = new Image();
		element.addEventListener("load", draw);
		element.src = fileName;
		images.set(fileName, element);
	}
	return images.get(fileName);
}

// Canvas x positions at which chunk column `chunkX` is visible: the map
// repeats east to west, so it can appear more than once.
function screenColumns(chunkX) {
	const wide = worldMap.meta.chunks_wide;
	const scale = pixelsPerChunk();
	const columns = [];
	for (const lap of [-1, 0, 1]) {
		const x = (chunkX + lap * wide - view.x) * scale + canvas.width / 2;
		if (x > -wide * scale && x < canvas.width + wide * scale) columns.push(x);
	}
	return columns;
}

const screenRow = (chunkY) =>
	(chunkY - view.y) * pixelsPerChunk() + canvas.height / 2;

function drawLayer(fileName) {
	const element = image(fileName);
	if (!element.complete || element.naturalWidth === 0) return;
	const { chunks_wide: wide, chunks_high: high } = worldMap.meta;
	const scale = pixelsPerChunk();
	for (const x of screenColumns(0)) {
		context.drawImage(element, x, screenRow(0), wide * scale, high * scale);
	}
}

function drawFactionLabels() {
	const size = 12 * window.devicePixelRatio;
	context.font = `600 ${size}px "Public Sans", system-ui, sans-serif`;
	context.textAlign = "center";
	context.textBaseline = "middle";
	for (const faction of worldMap.meta.factions ?? []) {
		if (!faction.name) continue;
		const y = screenRow(faction.capital_chunk[1] + 0.5) - size * 1.2;
		const width = context.measureText(faction.name).width + size * 0.7;
		for (const x of screenColumns(faction.capital_chunk[0] + 0.5)) {
			context.fillStyle = "rgba(16, 14, 38, 0.75)";
			context.fillRect(x - width / 2, y - size * 0.7, width, size * 1.4);
			context.fillStyle = "#fff";
			context.fillText(faction.name, x, y);
		}
	}
}

function drawSpawnPin() {
	const radius = 7 * window.devicePixelRatio;
	const y = screenRow(spawn.y + 0.5);
	for (const x of screenColumns(spawn.x + 0.5)) {
		context.beginPath();
		context.arc(x, y, radius, 0, Math.PI * 2);
		context.lineWidth = 5 * window.devicePixelRatio;
		context.strokeStyle = "#000";
		context.stroke();
		context.lineWidth = 2.5 * window.devicePixelRatio;
		context.strokeStyle = "#fff";
		context.stroke();
	}
}

function draw() {
	if (!worldMap) return;
	context.clearRect(0, 0, canvas.width, canvas.height);
	// Blur when shrinking, stay sharp when magnifying: chunks are the unit.
	context.imageSmoothingEnabled = view.zoom < 3;
	drawLayer(baseLayer);
	const overlays = worldMap.meta.overlays ?? [];
	for (const overlay of overlays) {
		if (activeOverlays.has(overlay.name)) drawLayer(overlay.file);
	}
	if (LABELLED_OVERLAYS.some((name) => activeOverlays.has(name))) {
		drawFactionLabels();
	}
	drawSpawnPin();
}

// ─── Controls ────────────────────────────────────────────────────────────────

function showLayer(fileName) {
	baseLayer = fileName;
	for (const button of document.querySelectorAll("#layers button")) {
		button.setAttribute(
			"aria-checked",
			String(button.dataset.layer === fileName),
		);
	}
	renderLegend();
	draw();
}

// One labelled row of base-layer buttons per group in map.json.
function renderLayerButtons() {
	const container = document.getElementById("layers");
	for (const group of worldMap.meta.layer_groups ?? []) {
		const row = document.createElement("div");
		row.className = "layer-group";
		const name = document.createElement("span");
		name.className = "layer-group-name";
		name.textContent = group.name;
		row.append(name);
		for (const fileName of group.layers) {
			const button = document.createElement("button");
			button.type = "button";
			button.role = "radio";
			button.dataset.layer = fileName;
			button.textContent = layerLabel(fileName);
			button.addEventListener("click", () => showLayer(fileName));
			row.append(button);
		}
		container.append(row);
	}
}

function renderOverlayToggles() {
	const container = document.getElementById("overlays");
	for (const overlay of worldMap.meta.overlays ?? []) {
		const label = document.createElement("label");
		const checkbox = document.createElement("input");
		checkbox.type = "checkbox";
		checkbox.addEventListener("change", () => {
			if (checkbox.checked) activeOverlays.add(overlay.name);
			else activeOverlays.delete(overlay.name);
			renderLegend();
			draw();
		});
		label.append(checkbox, ` ${overlay.name}`);
		container.append(label);
	}
}

const swatch = (rgb, text) =>
	`<span class="legend-item"><span class="swatch" style="background: rgb(${rgb.join(",")})"></span>${text}</span>`;

// What the colours on screen mean, for the overlays that are switched on.
function renderLegend() {
	const { meta } = worldMap;
	const parts = [];
	if (baseLayer.startsWith("lifegen_")) {
		parts.push(
			'<span class="legend-item">0 <span class="swatch ramp"></span> 1</span>',
		);
	}
	if (activeOverlays.has("Factions")) {
		parts.push(
			'<span class="legend-item">Coloured: held by a state. Clear: unclaimed or uninhabited. White dot: capital.</span>',
		);
	}
	if (activeOverlays.has("Settlements")) {
		for (const dot of meta.settlement_dots ?? []) {
			if (dot.size !== "Ruins") parts.push(swatch(dot.rgb, dot.size));
		}
	}
	if (activeOverlays.has("Roads")) {
		for (const road of meta.road_colours ?? []) {
			parts.push(swatch(road.rgb, road.kind));
		}
	}
	if (activeOverlays.has("Trade")) {
		parts.push(
			'<span class="legend-item">Trade: line from a settlement to its market <span class="swatch ramp"></span> brighter is richer</span>',
		);
	}
	document.getElementById("legend").innerHTML = parts.join("");
}

// A press that does not move is a click (set the spawn point); one that
// moves pans the map.
let press = null;
canvas.addEventListener("pointerdown", (event) => {
	if (!worldMap) return;
	canvas.setPointerCapture(event.pointerId);
	press = { x: event.offsetX, y: event.offsetY, dragged: false };
});
canvas.addEventListener("pointermove", (event) => {
	if (!press) return;
	const [dx, dy] = [event.offsetX - press.x, event.offsetY - press.y];
	if (!press.dragged && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
	press.dragged = true;
	const chunksPerCssPixel =
		canvas.width / canvas.clientWidth / pixelsPerChunk();
	view.x -= dx * chunksPerCssPixel;
	view.y -= dy * chunksPerCssPixel;
	press.x = event.offsetX;
	press.y = event.offsetY;
	constrainView();
	draw();
});
canvas.addEventListener("pointerup", (event) => {
	if (press && !press.dragged) {
		const chunk = chunkUnder(event.offsetX, event.offsetY);
		moveSpawn(chunk.x, chunk.y);
	}
	press = null;
});
canvas.addEventListener("pointercancel", () => {
	press = null;
});
// Trackpad pinch arrives as a wheel event with ctrlKey set. A plain wheel is
// left alone so the page still scrolls.
canvas.addEventListener(
	"wheel",
	(event) => {
		if (!worldMap || !event.ctrlKey) return;
		event.preventDefault();
		zoomAt(Math.exp(-event.deltaY * 0.01), event.offsetX, event.offsetY);
	},
	{ passive: false },
);

const zoomAtCentre = (factor) =>
	zoomAt(factor, canvas.clientWidth / 2, canvas.clientHeight / 2);
document
	.getElementById("zoomIn")
	.addEventListener("click", () => zoomAtCentre(ZOOM_STEP));
document
	.getElementById("zoomOut")
	.addEventListener("click", () => zoomAtCentre(1 / ZOOM_STEP));
document.getElementById("zoomReset").addEventListener("click", () => {
	Object.assign(view, { x: worldMap.meta.chunks_wide / 2, zoom: 1 });
	constrainView();
	draw();
});

mapFrame.addEventListener("keydown", (event) => {
	if (!worldMap) return;
	const step = event.shiftKey ? 16 : 1;
	const moves = {
		ArrowLeft: [-step, 0],
		ArrowRight: [step, 0],
		ArrowUp: [0, -step],
		ArrowDown: [0, step],
	};
	if (moves[event.key]) {
		event.preventDefault();
		moveSpawn(spawn.x + moves[event.key][0], spawn.y + moves[event.key][1]);
	} else if (event.key === "+" || event.key === "=") {
		zoomAtCentre(ZOOM_STEP);
	} else if (event.key === "-") {
		zoomAtCentre(1 / ZOOM_STEP);
	}
});

// Launch the web build in the dialog, spawning at the centre of the chosen chunk.
const play = document.getElementById("play");
const playFrame = document.getElementById("playFrame");
document.getElementById("playButton").addEventListener("click", () => {
	const worldOrigin = `${spawn.x + 0.5},${spawn.y + 0.5}`;
	document.getElementById("playTitle").textContent =
		`Chunk ${spawn.x}, ${spawn.y}`;
	playFrame.src = `/play/index.html?origin=${worldOrigin}`;
	play.showModal();
});
document
	.getElementById("playClose")
	.addEventListener("click", () => play.close());
// Unload the game when the dialog closes so it stops running.
play.addEventListener("close", () => playFrame.removeAttribute("src"));

async function loadWorldMap() {
	try {
		const [meta, chunks] = await Promise.all([
			fetch("map.json").then((response) => response.json()),
			fetch("chunks.bin").then((response) => response.arrayBuffer()),
		]);
		worldMap = { meta, chunks: new Uint8Array(chunks) };
	} catch {
		document.getElementById("mapStatus").textContent =
			"Map data not found. Export it with: margins_grip export site-map site/dist/map";
		return;
	}
	Object.assign(view, {
		x: worldMap.meta.chunks_wide / 2,
		y: worldMap.meta.chunks_high / 2,
	});
	renderLayerButtons();
	renderOverlayToggles();
	resizeCanvas();
	showLayer(baseLayer);
	renderSpawnReadout();
}

window.addEventListener("resize", resizeCanvas);
loadWorldMap();
