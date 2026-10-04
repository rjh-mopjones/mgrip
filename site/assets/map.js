// Map page: pick a spawn chunk on the world map and launch the web build there.
//
// Map data is exported by the Rust CLI (`margins_grip export site-map`) into
// this page's directory: one PNG per layer (macromap, heightmap, …), plus
// chunks.bin with one record per chunk as described by map.json.

const DEFAULT_SPAWN_CHUNK = { x: 440, y: 220 };
const DEFAULT_LAYER_STEM = "macromap";
const spawn = { ...DEFAULT_SPAWN_CHUNK };
let worldMap = null;

// 'InnerTerminus' -> 'Inner terminus'
const readable = (name) =>
	name
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.replace(/ ([A-Z])/g, (_match, letter) => ` ${letter.toLowerCase()}`);

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

function showLayer(fileName) {
	document.getElementById("mapImage").src = fileName;
	document.getElementById("factionLabels").hidden =
		!LABELLED_LAYERS.includes(fileName);
	for (const button of document.querySelectorAll("#layers button")) {
		button.setAttribute(
			"aria-checked",
			String(button.dataset.layer === fileName),
		);
	}
}

// One labelled row of layer buttons per group in map.json.
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
	showLayer(`${DEFAULT_LAYER_STEM}.png`);
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

// Layers drawn over the faction map get the names of the authored states.
const LABELLED_LAYERS = [
	"lifegen_factions.png",
	"lifegen_settlements.png",
	"lifegen_roads.png",
	"lifegen_trade.png",
];

function renderFactionLabels() {
	const { meta } = worldMap;
	const container = document.getElementById("factionLabels");
	for (const faction of meta.factions ?? []) {
		if (!faction.name) continue;
		const label = document.createElement("span");
		label.textContent = faction.name;
		label.style.left = `${((faction.capital_chunk[0] + 0.5) / meta.chunks_wide) * 100}%`;
		label.style.top = `${((faction.capital_chunk[1] + 0.5) / meta.chunks_high) * 100}%`;
		container.append(label);
	}
}

// "City, 3 chunks away": the settlement closest to the spawn chunk.
function nearestSettlementText() {
	const { settlements, settlement_sizes: sizes } = worldMap.meta;
	if (!settlements?.length) return "none";
	let nearest = null;
	let nearestDistance = Number.POSITIVE_INFINITY;
	for (const settlement of settlements) {
		const distance = Math.hypot(
			settlement[0] - spawn.x,
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

const chunkText = () => `${spawn.x}, ${spawn.y}`;

function renderSpawn() {
	if (!worldMap) return;
	const { meta } = worldMap;
	const chunk = chunkAt(spawn.x, spawn.y);
	const pin = document.getElementById("pin");
	pin.style.left = `${((spawn.x + 0.5) / meta.chunks_wide) * 100}%`;
	pin.style.top = `${((spawn.y + 0.5) / meta.chunks_high) * 100}%`;
	document.getElementById("spawnZone").textContent = readable(chunk.zone);
	document.getElementById("spawnChunk").textContent = chunkText();
	document.getElementById("spawnLight").textContent = chunk.light.toFixed(2);
	document.getElementById("spawnBiome").textContent = readable(chunk.biome);
	document.getElementById("spawnProvince").textContent = provinceText(
		chunk.provinceId,
	);
	document.getElementById("spawnFaction").textContent = factionText(
		chunk.provinceId,
	);
	document.getElementById("spawnSettlement").textContent =
		nearestSettlementText();
	document.getElementById("spawnSeed").textContent = meta.seed;
}

function moveSpawn(x, y) {
	if (!worldMap) return;
	const { meta } = worldMap;
	spawn.x = Math.max(0, Math.min(meta.chunks_wide - 1, Math.floor(x)));
	spawn.y = Math.max(0, Math.min(meta.chunks_high - 1, Math.floor(y)));
	renderSpawn();
}

async function loadWorldMap() {
	try {
		const [meta, chunks] = await Promise.all([
			fetch("map.json").then((response) => response.json()),
			fetch("chunks.bin").then((response) => response.arrayBuffer()),
		]);
		worldMap = { meta, chunks: new Uint8Array(chunks) };
		renderFactionLabels();
		renderLayerButtons();
		renderSpawn();
	} catch {
		document.getElementById("mapStatus").textContent =
			"Map data not found. Export it with: margins_grip export site-map site/dist/map";
	}
}

const mapFrame = document.getElementById("mapFrame");
mapFrame.addEventListener("click", (event) => {
	if (!worldMap) return;
	const box = mapFrame.getBoundingClientRect();
	moveSpawn(
		((event.clientX - box.left) / box.width) * worldMap.meta.chunks_wide,
		((event.clientY - box.top) / box.height) * worldMap.meta.chunks_high,
	);
});
mapFrame.addEventListener("keydown", (event) => {
	const step = event.shiftKey ? 16 : 1;
	const moves = {
		ArrowLeft: [-step, 0],
		ArrowRight: [step, 0],
		ArrowUp: [0, -step],
		ArrowDown: [0, step],
	};
	const move = moves[event.key];
	if (!move) return;
	event.preventDefault();
	moveSpawn(spawn.x + move[0], spawn.y + move[1]);
});

// Launch the web build in the dialog, spawning at the centre of the chosen chunk.
const play = document.getElementById("play");
const playFrame = document.getElementById("playFrame");
document.getElementById("playButton").addEventListener("click", () => {
	const worldOrigin = `${spawn.x + 0.5},${spawn.y + 0.5}`;
	document.getElementById("playTitle").textContent = `Chunk ${chunkText()}`;
	playFrame.src = `/play/index.html?origin=${worldOrigin}`;
	play.showModal();
});
document
	.getElementById("playClose")
	.addEventListener("click", () => play.close());
// Unload the game when the dialog closes so it stops running.
play.addEventListener("close", () => playFrame.removeAttribute("src"));

loadWorldMap();
