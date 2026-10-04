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

// "12: 18 provinces", or "unclaimed" / "uninhabited" for land no faction holds.
function factionText(provinceId) {
	const province = worldMap.meta.provinces?.[provinceId - 1];
	if (!province) return "none";
	const faction = worldMap.meta.factions?.[province.faction - 1];
	if (!faction) return province.state;
	return `${province.faction}: ${faction.provinces} provinces`;
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
