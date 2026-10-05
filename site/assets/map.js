// Map page: a campaign-style map of the world. Provinces can be hovered and
// selected, map modes recolour them, and a spawn chunk can be picked to
// launch the web build there.
//
// Map data is exported by the Rust CLI (`margins_grip export site-map`) into
// this page's directory: one
// PNG per generation layer, chunks.bin with one record per chunk, map.json
// with the province, faction and settlement tables, network.json with the
// roads, trade flows and river courses, and world.mgmacro, the macro pack
// that sharper terrain is generated from as the view zooms in.
//
// Drawing follows the usual campaign-map scheme: one texture holds the
// province id of every chunk, another holds one colour per province. A map
// mode, a hover or a selection only rewrites the small colour table.

const DEFAULT_SPAWN_CHUNK = { x: 440, y: 220 };
const TERRAIN_IMAGE = "macromap.png";
const RELIEF_IMAGE = "relief.png";
const SEA_IMAGE = "sea.png";
const MAX_ZOOM = 64;
const ZOOM_STEP = 1.5;
const WHEEL_ZOOM_RATE = 0.0015;
/// A press that moves further than this many pixels is a drag, not a click.
const DRAG_THRESHOLD_PX = 4;

const RAMP = [
	[16, 14, 38],
	[84, 38, 104],
	[186, 70, 104],
	[244, 150, 74],
	[252, 240, 190],
];
const UNINHABITED_TINT = [34, 30, 52, 150];
const NO_TINT = [0, 0, 0, 0];

// Colour of a 0-to-1 score on the ramp, as [r, g, b, alpha] bytes.
function rampColour(score) {
	const position = Math.max(0, Math.min(1, score)) * (RAMP.length - 1);
	const lower = Math.min(RAMP.length - 2, Math.floor(position));
	const blend = position - lower;
	const channel = (index) =>
		RAMP[lower][index] + (RAMP[lower + 1][index] - RAMP[lower][index]) * blend;
	return [channel(0), channel(1), channel(2), 235];
}

// A distinct colour per faction. Hues run blue through magenta and red to
// yellow, skipping green.
function factionColour(factionId) {
	// Scramble the id so neighbouring states get unrelated colours.
	const hash = Math.imul(factionId, 2654435761) >>> 0;
	const hue = 200 + (((hash >>> 8) & 0xff) / 255) * 220;
	const saturation = 0.45 + (((hash >>> 16) & 0xff) / 255) * 0.3;
	const value = 0.65 + ((hash >>> 24) / 255) * 0.3;
	const channel = (offset) => {
		const k = (offset + hue / 60) % 6;
		const shade = Math.max(0, Math.min(1, Math.min(k, 4 - k)));
		return (value - value * saturation * shade) * 255;
	};
	return [channel(5), channel(3), channel(1), 175];
}

// Each map mode gives one colour per province, laid over the terrain.
const MAP_MODES = [
	{
		id: "political",
		name: "Political",
		// A darker band inside each state's border.
		stateBands: true,
		legend:
			"Coloured: held by a state. Plain terrain: unclaimed. Dark: uninhabited.",
		colour: (province) => {
			if (province.faction) return factionColour(province.faction);
			return province.state === "uninhabited" ? UNINHABITED_TINT : NO_TINT;
		},
	},
	{ id: "terrain", name: "Terrain", colour: () => NO_TINT },
	{
		id: "habitability",
		name: "Habitability",
		ramp: "Province habitability",
		colour: (province) => rampColour(province.habitability),
	},
	{
		id: "light",
		name: "Light",
		ramp: "Province light level",
		colour: (province) => rampColour(province.light),
	},
	{
		id: "resources",
		name: "Resources",
		ramp: "Province resource desirability",
		colour: (province) => rampColour(province.resources),
	},
];

const spawn = { ...DEFAULT_SPAWN_CHUNK };
let worldMap = null;
let renderer = null;
let mapMode = MAP_MODES[0];
// File name of the raw layer on show, or null while a map mode is active.
let rawLayer = null;
let hoveredProvince = 0;
let selectedProvince = 0;
// Centre of the view in chunk coordinates; zoom 1 fits the whole world.
const view = { x: 512, y: 256, zoom: 1 };

const canvas = document.getElementById("mapCanvas");
const labelCanvas = document.getElementById("labelCanvas");
const labelContext = labelCanvas.getContext("2d");
const mapFrame = document.getElementById("mapFrame");

// 'InnerTerminus' -> 'Inner terminus'
const readable = (name) =>
	name
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.replace(/ ([A-Z])/g, (_match, letter) => ` ${letter.toLowerCase()}`);

// 'light_level.png' -> 'Light level', 'lifegen_habitability.png' -> 'Habitability'
function layerLabel(fileName) {
	const stem = fileName.replace(/\.png$/, "");
	const words = stem
		.replace(/^lifegen_/, "")
		.replace(/_/g, " ")
		.toLowerCase();
	return words.charAt(0).toUpperCase() + words.slice(1);
}

// ─── World data ──────────────────────────────────────────────────────────────

function chunkAt(x, y) {
	const { meta, chunks } = worldMap;
	const offset = (y * meta.chunks_wide + x) * meta.chunk_fields.length;
	return {
		light: chunks[offset] / 255,
		zone: meta.zones[chunks[offset + 1]],
		biome: meta.biomes[chunks[offset + 2]],
		// 16-bit id, low byte first; 0 = no province (sea).
		provinceId: chunks[offset + 3] | (chunks[offset + 4] << 8),
	};
}

// Province id of a cell; the map joins east to west.
function provinceOfCell(x, y) {
	const high = worldMap.meta.chunks_high;
	return chunkAt(wrapColumn(x), Math.max(0, Math.min(high - 1, y))).provinceId;
}

// Whether a point given in chunks is liquid sea. The sea is known on a
// finer grid than provinces are (sea.png).
function isSea(x, y) {
	const { pixels, width, height } = worldMap.sea;
	const perChunk = width / worldMap.meta.chunks_wide;
	const column = ((Math.floor(x * perChunk) % width) + width) % width;
	const row = Math.max(0, Math.min(height - 1, Math.floor(y * perChunk)));
	return pixels[row * width + column] > 127;
}

// The eight cells round a cell, nearest first as the shader lists them.
const CELLS_BESIDE = [
	[1, 0],
	[1, 1],
	[0, 1],
	[-1, 1],
	[-1, 0],
	[-1, -1],
	[0, -1],
	[1, -1],
];

// Province at a point given in chunks. Where a cell's corner pokes into a
// neighbouring province the corner belongs to that province, which turns
// stair-stepped borders into diagonals. Land in a chunk with no province
// belongs to a province next door. The shader's provinceAt applies the same
// rules, so what is clicked is what is drawn.
function provinceAtPoint(x, y) {
	if (isSea(x, y)) return 0;
	const [cellX, cellY] = [Math.floor(x), Math.floor(y)];
	const own = provinceOfCell(cellX, cellY);
	if (own === 0) {
		for (const [dx, dy] of CELLS_BESIDE) {
			const beside = provinceOfCell(cellX + dx, cellY + dy);
			if (beside !== 0) return beside;
		}
		return 0;
	}
	const [inX, inY] = [x - cellX, y - cellY];
	if (Math.min(inX, 1 - inX) + Math.min(inY, 1 - inY) >= 0.5) return own;
	const across = provinceOfCell(cellX + (inX < 0.5 ? -1 : 1), cellY);
	const along = provinceOfCell(cellX, cellY + (inY < 0.5 ? -1 : 1));
	return across === along && across !== 0 ? across : own;
}

// The red channel of an image, for reading it as data.
async function imagePixels(fileName) {
	const element = new Image();
	element.src = fileName;
	await element.decode();
	const surface = new OffscreenCanvas(
		element.naturalWidth,
		element.naturalHeight,
	);
	const context = surface.getContext("2d");
	context.drawImage(element, 0, 0);
	const rgba = context.getImageData(0, 0, surface.width, surface.height).data;
	const pixels = new Uint8Array(surface.width * surface.height);
	for (let index = 0; index < pixels.length; index++)
		pixels[index] = rgba[index * 4];
	return { pixels, width: surface.width, height: surface.height };
}

const provinceById = (provinceId) => worldMap.meta.provinces[provinceId - 1];
const factionById = (factionId) => worldMap.meta.factions[factionId - 1];
const factionName = (factionId) => factionById(factionId).name;

// Wrap a chunk column onto the map.
function wrapColumn(x) {
	const wide = worldMap.meta.chunks_wide;
	return ((Math.floor(x) % wide) + wide) % wide;
}

// Chunks between two columns, the short way round the map.
function columnDistance(a, b) {
	const direct = Math.abs(a - b);
	return Math.min(direct, worldMap.meta.chunks_wide - direct);
}

// Where each state's name goes: the middle of its territory. The map is a
// ring east to west, so the mean column is taken round the ring.
function stateLabelPositions() {
	const { meta } = worldMap;
	const sums = meta.factions.map(() => ({ cos: 0, sin: 0, y: 0, count: 0 }));
	const turn = (Math.PI * 2) / meta.chunks_wide;
	for (let y = 0; y < meta.chunks_high; y++) {
		for (let x = 0; x < meta.chunks_wide; x++) {
			const { provinceId } = chunkAt(x, y);
			const factionId = provinceId && provinceById(provinceId).faction;
			if (!factionId) continue;
			const sum = sums[factionId - 1];
			sum.cos += Math.cos(x * turn);
			sum.sin += Math.sin(x * turn);
			sum.y += y;
			sum.count += 1;
		}
	}
	return sums.map((sum) => ({
		x: wrapColumn(Math.atan2(sum.sin, sum.cos) / turn),
		y: sum.y / Math.max(1, sum.count),
	}));
}

// "2 cities, 3 villages": the settlements standing in a province.
function settlementSummary(provinceId) {
	const { settlements, settlement_sizes: sizes } = worldMap.meta;
	const counts = sizes.map(() => 0);
	for (const settlement of settlements) {
		if (settlement[3] === provinceId) counts[settlement[2]] += 1;
	}
	const plural = (name) =>
		name.endsWith("y") ? `${name.slice(0, -1)}ies` : `${name}s`;
	const parts = counts
		.map((count, index) => {
			const name = sizes[index].toLowerCase();
			return count && `${count} ${count === 1 ? name : plural(name)}`;
		})
		.filter(Boolean);
	return parts.length ? parts.join(", ") : "none";
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
	return `${nearest[4]} (${sizes[nearest[2]].toLowerCase()}), ${where}`;
}

// ─── Readouts ────────────────────────────────────────────────────────────────

function renderSpawnReadout() {
	const chunk = chunkAt(spawn.x, spawn.y);
	const set = (id, text) => {
		document.getElementById(id).textContent = text;
	};
	set("spawnZone", readable(chunk.zone));
	set("spawnChunk", `${spawn.x}, ${spawn.y}`);
	set("spawnLight", chunk.light.toFixed(2));
	set("spawnBiome", readable(chunk.biome));
	set("spawnSettlement", nearestSettlementText());
	set("spawnSeed", worldMap.meta.seed);
}

// Who holds a province: a state, or the reason no state does.
function ownerText(province) {
	if (!province.faction) {
		return province.state === "uninhabited" ? "Uninhabited" : "Unclaimed";
	}
	const faction = factionById(province.faction);
	const capital = faction.capital_name
		? `, capital ${faction.capital_name}`
		: "";
	return `${factionName(province.faction)}${capital} (${faction.provinces} provinces)`;
}

function renderProvincePanel() {
	const panel = document.getElementById("provincePanel");
	panel.hidden = selectedProvince === 0;
	if (selectedProvince === 0) return;
	const province = provinceById(selectedProvince);
	const features = [
		province.coastal && "coastal",
		province.major_river && "major river",
	].filter(Boolean);
	const facts = [
		["Province", String(selectedProvince)],
		["Held by", ownerText(province)],
		["Biome", readable(province.biome)],
		["Area", `${province.area_chunks} chunks`],
		["Habitability", province.habitability.toFixed(2)],
		["Light level", province.light.toFixed(2)],
		["Resources", province.resources.toFixed(2)],
		["Features", features.length ? features.join(", ") : "none"],
		["Settlements", settlementSummary(selectedProvince)],
	];
	document.getElementById("panelTitle").textContent = province.name;
	const list = document.getElementById("panelFacts");
	list.replaceChildren();
	for (const [name, value] of facts) {
		const row = document.createElement("div");
		const term = document.createElement("dt");
		const detail = document.createElement("dd");
		term.textContent = name;
		detail.textContent = value;
		row.append(term, detail);
		list.append(row);
	}
}

function renderHoverReadout() {
	const readout = document.getElementById("mapHover");
	readout.hidden = hoveredProvince === 0;
	if (hoveredProvince === 0) return;
	const province = provinceById(hoveredProvince);
	const owner = province.faction
		? factionName(province.faction)
		: province.state;
	readout.textContent = `${province.name}, ${owner}`;
}

const swatch = (rgb, text) =>
	`<span class="legend-item"><span class="swatch" style="background: rgb(${rgb.join(",")})"></span>${text}</span>`;
const rampLegend = (text) =>
	`<span class="legend-item">${text}: 0 <span class="swatch ramp"></span> 1</span>`;

// What the colours on screen mean.
function renderLegend() {
	const { meta } = worldMap;
	const parts = [];
	if (rawLayer?.startsWith("lifegen_")) {
		parts.push(rampLegend(layerLabel(rawLayer)));
	} else if (!rawLayer && mapMode.ramp) {
		parts.push(rampLegend(mapMode.ramp));
	} else if (!rawLayer && mapMode.legend) {
		parts.push(`<span class="legend-item">${mapMode.legend}</span>`);
	}
	if (!rawLayer && shown.has("Settlements")) {
		meta.settlement_sizes.forEach((size, index) => {
			if (size !== "Ruins")
				parts.push(swatch(SETTLEMENT_STYLES[index].rgb, size));
		});
	}
	if (!rawLayer && shown.has("Roads")) {
		worldMap.roadKinds.forEach((kind, index) => {
			parts.push(swatch(ROAD_STYLES[index].rgb, kind));
		});
	}
	if (!rawLayer && shown.has("Trade")) {
		parts.push(
			'<span class="legend-item">Trade: line from a settlement to its market <span class="swatch ramp"></span> brighter is richer</span>',
		);
	}
	if (!rawLayer) {
		parts.push(
			'<span class="legend-item">Smaller settlements and roads appear as you zoom in.</span>',
		);
	}
	document.getElementById("legend").innerHTML = parts.join("");
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

// Province under a point of the canvas (CSS pixels); 0 for sea or off the map.
function provinceUnder(cssX, cssY) {
	const chunk = chunkUnder(cssX, cssY);
	if (chunk.y < 0 || chunk.y >= worldMap.meta.chunks_high) return 0;
	return provinceAtPoint(chunk.x, chunk.y);
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
	for (const surface of [canvas, labelCanvas]) {
		surface.width = width;
		surface.height = width / 2;
	}
	if (worldMap) {
		constrainView();
		draw();
	}
}

// ─── Renderer ────────────────────────────────────────────────────────────────

const VERTEX_SHADER = `#version 300 es
// One triangle that covers the canvas.
void main() {
	vec2 corner = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
	gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp int;

uniform sampler2D uBase;        // terrain, or a raw layer
uniform sampler2D uProvinceIds; // one texel per chunk: province id, low byte in r
uniform sampler2D uProvinces;   // row 0: colour per province; row 1: owning faction
uniform sampler2D uRelief;      // hillshade: 0.5 is flat ground
uniform sampler2D uSea;         // liquid sea, finer than one cell per chunk
uniform vec2 uBaseOrigin;    // chunk at the base image's top-left corner
uniform vec2 uBaseSize;      // chunks the base image covers
uniform float uBaseShaded;   // 1 if the base image already has relief shading
uniform vec2 uCanvas;        // pixels
uniform vec2 uWorld;         // chunks
uniform vec2 uCentre;        // chunk at the middle of the canvas
uniform float uPixelsPerChunk;
uniform float uPixelRatio;
uniform float uProvinceLayer; // 0 hides tints and borders (raw layers)
uniform float uStateBands;    // 1 draws a darker band inside state borders
uniform int uHovered;
uniform int uSelected;

out vec4 colour;

const vec3 BORDER = vec3(0.063, 0.055, 0.149);
const vec3 OFF_MAP = vec3(0.933, 0.945, 0.957);
// How strongly the hillshade lightens and darkens the land.
const float RELIEF_STRENGTH = 0.9;
// The sea is lighter within this many chunks of land.
const float SHELF_CHUNKS = 3.0;
// Width, in pixels, of the darker band inside a state's border.
const float STATE_BAND_PIXELS = 5.0;
// Eight directions round a circle, for finding borders.
const vec2 RING[8] = vec2[8](
	vec2(1.0, 0.0), vec2(0.707, 0.707), vec2(0.0, 1.0), vec2(-0.707, 0.707),
	vec2(-1.0, 0.0), vec2(-0.707, -0.707), vec2(0.0, -1.0), vec2(0.707, -0.707)
);

int unpackId(vec4 texel) {
	return int(texel.r * 255.0 + 0.5) + int(texel.g * 255.0 + 0.5) * 256;
}

// The map joins east to west.
int provinceOfCell(ivec2 cell) {
	int wide = int(uWorld.x);
	cell.x = ((cell.x % wide) + wide) % wide;
	cell.y = clamp(cell.y, 0, int(uWorld.y) - 1);
	return unpackId(texelFetch(uProvinceIds, cell, 0));
}

// The sea is known on a finer grid than provinces are, so the coast is
// drawn from it and not from the province ids.
bool isSea(vec2 chunk) {
	return texture(uSea, chunk / uWorld).r > 0.5;
}

// Province at a point. Province ids come one per chunk, which would give
// stair-stepped borders; where a cell's corner pokes into a neighbouring
// province, the corner is given to that province, turning steps into
// diagonals. Land in a chunk that has no province (its middle is sea)
// belongs to a province next door. provinceAtPoint in map.js applies the
// same rules.
int provinceAt(vec2 chunk) {
	if (isSea(chunk)) return 0;
	ivec2 cell = ivec2(floor(chunk));
	int own = provinceOfCell(cell);
	if (own == 0) {
		for (int direction = 0; direction < 8; direction++) {
			int beside = provinceOfCell(cell + ivec2(round(RING[direction] * 1.3)));
			if (beside != 0) return beside;
		}
		return 0;
	}
	vec2 inCell = fract(chunk);
	vec2 toEdge = min(inCell, 1.0 - inCell);
	if (toEdge.x + toEdge.y >= 0.5) return own;
	ivec2 side = ivec2(inCell.x < 0.5 ? -1 : 1, inCell.y < 0.5 ? -1 : 1);
	int across = provinceOfCell(cell + ivec2(side.x, 0));
	int along = provinceOfCell(cell + ivec2(0, side.y));
	return (across == along && across != 0) ? across : own;
}

int ownerOf(int province) {
	return unpackId(texelFetch(uProvinces, ivec2(province, 1), 0));
}

void main() {
	vec2 pixel = vec2(gl_FragCoord.x, uCanvas.y - gl_FragCoord.y);
	vec2 chunk = uCentre + (pixel - uCanvas * 0.5) / uPixelsPerChunk;
	if (chunk.y < 0.0 || chunk.y >= uWorld.y) {
		colour = vec4(OFF_MAP, 1.0);
		return;
	}
	vec3 shade = texture(uBase, (chunk - uBaseOrigin) / uBaseSize).rgb;

	int province = provinceAt(chunk);
	if (province == 0 && uProvinceLayer > 0.0) {
		// A lighter shelf along the coast: the more land near a point of sea,
		// the lighter it is.
		float land = 0.0;
		for (int direction = 0; direction < 8; direction++) {
			vec2 reach = RING[direction] * SHELF_CHUNKS;
			if (!isSea(chunk + reach * 0.5)) land += 0.6;
			if (!isSea(chunk + reach)) land += 0.4;
		}
		shade = mix(shade, shade * 1.25 + 0.06, min(land / 4.0, 1.0));
	}
	if (province != 0 && uProvinceLayer > 0.0) {
		// Tint the terrain rather than cover it, so its texture shows through.
		vec4 tint = texelFetch(uProvinces, ivec2(province, 0), 0);
		float brightness = dot(shade, vec3(0.299, 0.587, 0.114));
		shade = mix(shade, tint.rgb * mix(0.45, 1.35, brightness), tint.a);
		// Hillshade over terrain and tint alike.
		if (uBaseShaded < 0.5) {
			float relief = texture(uRelief, chunk / uWorld).r - 0.5;
			shade *= 1.0 + relief * RELIEF_STRENGTH;
		}
		if (province == uSelected) {
			shade = mix(shade, vec3(1.0), 0.28);
		} else if (province == uHovered) {
			shade = mix(shade, vec3(1.0), 0.16);
		}

		// A point is on a border if another province lies within the border's
		// half-width of it. Counting how many of eight directions find one
		// gives a soft edge.
		int owner = ownerOf(province);
		float provinceHits = 0.0;
		float stateHits = 0.0;
		float outlineHits = 0.0;
		float bandHits = 0.0;
		for (int direction = 0; direction < 8; direction++) {
			vec2 reach = RING[direction] * uPixelRatio / uPixelsPerChunk;
			int thin = provinceAt(chunk + reach * 0.6);
			if (thin != province) provinceHits += 1.0;
			int thick = provinceAt(chunk + reach * 1.2);
			if (thick != province && (thick == 0 || ownerOf(thick) != owner)) stateHits += 1.0;
			if (province == uSelected && provinceAt(chunk + reach * 2.0) != province) outlineHits += 1.0;
			if (uStateBands > 0.0 && owner != 0) {
				int beyond = provinceAt(chunk + reach * STATE_BAND_PIXELS);
				if (beyond == 0 || ownerOf(beyond) != owner) bandHits += 1.0;
			}
		}
		// Province borders fade when zoomed far out; state borders stay.
		shade = mix(shade, tint.rgb * 0.45, min(bandHits / 4.0, 1.0) * 0.45);
		float provinceStrength = mix(0.25, 0.7, smoothstep(1.5, 5.0, uPixelsPerChunk / uPixelRatio));
		shade = mix(shade, BORDER, min(provinceHits / 3.0, 1.0) * provinceStrength);
		shade = mix(shade, BORDER, min(stateHits / 3.0, 1.0) * 0.9);
		shade = mix(shade, vec3(1.0), min(outlineHits / 3.0, 1.0));
	}

	colour = vec4(shade, 1.0);
}`;

// Sets up WebGL and returns the few operations the page needs, or null if
// WebGL 2 is not available.
function createRenderer() {
	const gl = canvas.getContext("webgl2", { antialias: false });
	if (!gl) return null;

	const compile = (type, source) => {
		const shader = gl.createShader(type);
		gl.shaderSource(shader, source);
		gl.compileShader(shader);
		if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
			throw new Error(gl.getShaderInfoLog(shader));
		}
		return shader;
	};
	const program = gl.createProgram();
	gl.attachShader(program, compile(gl.VERTEX_SHADER, VERTEX_SHADER));
	gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAGMENT_SHADER));
	gl.linkProgram(program);
	if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
		throw new Error(gl.getProgramInfoLog(program));
	}
	gl.useProgram(program);
	gl.bindVertexArray(gl.createVertexArray());
	const uniformLocations = new Map();
	const uniform = (name) => {
		if (!uniformLocations.has(name)) {
			uniformLocations.set(name, gl.getUniformLocation(program, name));
		}
		return uniformLocations.get(name);
	};

	const { meta, chunks } = worldMap;
	const [wide, high] = [meta.chunks_wide, meta.chunks_high];
	const provinceCount = meta.provinces.length;

	// Texture units: 0 base image, 1 province ids, 2 province table, 3 relief,
	// 4 sea.
	["uBase", "uProvinceIds", "uProvinces", "uRelief", "uSea"].forEach(
		(name, unit) => {
			gl.uniform1i(uniform(name), unit);
		},
	);

	const dataTexture = (unit) => {
		gl.activeTexture(gl.TEXTURE0 + unit);
		gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
		for (const parameter of [gl.TEXTURE_MIN_FILTER, gl.TEXTURE_MAG_FILTER]) {
			gl.texParameteri(gl.TEXTURE_2D, parameter, gl.NEAREST);
		}
		for (const parameter of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T]) {
			gl.texParameteri(gl.TEXTURE_2D, parameter, gl.CLAMP_TO_EDGE);
		}
	};

	// Province id per chunk, two bytes each.
	const ids = new Uint8Array(wide * high * 2);
	const fields = meta.chunk_fields.length;
	for (let cell = 0; cell < wide * high; cell++) {
		ids[cell * 2] = chunks[cell * fields + 3];
		ids[cell * 2 + 1] = chunks[cell * fields + 4];
	}
	gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
	dataTexture(1);
	gl.texImage2D(
		gl.TEXTURE_2D,
		0,
		gl.RG8,
		wide,
		high,
		0,
		gl.RG,
		gl.UNSIGNED_BYTE,
		ids,
	);

	// Province table. Index 0 is "no province".
	const table = new Uint8Array((provinceCount + 1) * 2 * 4);
	meta.provinces.forEach((province, index) => {
		const owner = (provinceCount + 1 + index + 1) * 4;
		table[owner] = province.faction & 0xff;
		table[owner + 1] = province.faction >> 8;
	});
	dataTexture(2);

	// Images are loaded once and kept; a blank texel stands in until then.
	const imageTextures = new Map();
	const imageTexture = (fileName) => {
		if (imageTextures.has(fileName)) return imageTextures.get(fileName);
		const texture = gl.createTexture();
		imageTextures.set(fileName, texture);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, texture);
		gl.texImage2D(
			gl.TEXTURE_2D,
			0,
			gl.RGBA,
			1,
			1,
			0,
			gl.RGBA,
			gl.UNSIGNED_BYTE,
			new Uint8Array(4),
		);
		const element = new Image();
		element.addEventListener("load", () => {
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture);
			gl.texImage2D(
				gl.TEXTURE_2D,
				0,
				gl.RGBA,
				gl.RGBA,
				gl.UNSIGNED_BYTE,
				element,
			);
			gl.generateMipmap(gl.TEXTURE_2D);
			gl.texParameteri(
				gl.TEXTURE_2D,
				gl.TEXTURE_MIN_FILTER,
				gl.LINEAR_MIPMAP_LINEAR,
			);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
			draw();
		});
		element.src = fileName;
		return texture;
	};

	return {
		// One colour per province, from `colourOf(province) -> [r, g, b, a]`.
		setProvinceColours(colourOf) {
			meta.provinces.forEach((province, index) => {
				table.set(colourOf(province).map(Math.round), (index + 1) * 4);
			});
			gl.activeTexture(gl.TEXTURE2);
			gl.texImage2D(
				gl.TEXTURE_2D,
				0,
				gl.RGBA,
				provinceCount + 1,
				2,
				0,
				gl.RGBA,
				gl.UNSIGNED_BYTE,
				table,
			);
		},
		// A texture for one terrain tile, from its RGBA pixels.
		tileTexture(pixels, size) {
			const texture = gl.createTexture();
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture);
			gl.texImage2D(
				gl.TEXTURE_2D,
				0,
				gl.RGBA,
				size,
				size,
				0,
				gl.RGBA,
				gl.UNSIGNED_BYTE,
				pixels,
			);
			for (const parameter of [gl.TEXTURE_MIN_FILTER, gl.TEXTURE_MAG_FILTER]) {
				gl.texParameteri(gl.TEXTURE_2D, parameter, gl.LINEAR);
			}
			for (const parameter of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T]) {
				gl.texParameteri(gl.TEXTURE_2D, parameter, gl.CLAMP_TO_EDGE);
			}
			return texture;
		},
		deleteTexture(texture) {
			gl.deleteTexture(texture);
		},
		// Draws the base image over the whole canvas, then each of `tiles`
		// ({ texture, x, y, span } in chunks) over its own part of it.
		draw({ baseImage, showProvinces, stateBands, tiles }) {
			gl.viewport(0, 0, canvas.width, canvas.height);
			// Look the texture up first: creating one changes the active unit.
			const relief = imageTexture(RELIEF_IMAGE);
			const sea = imageTexture(SEA_IMAGE);
			gl.activeTexture(gl.TEXTURE4);
			gl.bindTexture(gl.TEXTURE_2D, sea);
			gl.activeTexture(gl.TEXTURE3);
			gl.bindTexture(gl.TEXTURE_2D, relief);
			gl.uniform1f(uniform("uStateBands"), stateBands ? 1 : 0);
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, imageTexture(baseImage));
			gl.uniform2f(uniform("uCanvas"), canvas.width, canvas.height);
			gl.uniform2f(uniform("uWorld"), wide, high);
			gl.uniform2f(uniform("uCentre"), view.x, view.y);
			gl.uniform1f(uniform("uPixelsPerChunk"), pixelsPerChunk());
			gl.uniform1f(uniform("uPixelRatio"), window.devicePixelRatio);
			gl.uniform1f(uniform("uProvinceLayer"), showProvinces ? 1 : 0);
			gl.uniform1i(uniform("uHovered"), hoveredProvince);
			gl.uniform1i(uniform("uSelected"), selectedProvince);
			gl.uniform2f(uniform("uBaseOrigin"), 0, 0);
			gl.uniform2f(uniform("uBaseSize"), wide, high);
			gl.uniform1f(uniform("uBaseShaded"), 0);
			gl.drawArrays(gl.TRIANGLES, 0, 3);

			// Tiles carry their own relief shading. Each is drawn by the same
			// shader, confined to the part of the canvas it covers. Edges are
			// rounded to whole pixels so neighbouring tiles meet exactly.
			const scale = pixelsPerChunk();
			const column = (x) => Math.round((x - view.x) * scale + canvas.width / 2);
			const row = (y) => Math.round((y - view.y) * scale + canvas.height / 2);
			gl.enable(gl.SCISSOR_TEST);
			gl.uniform1f(uniform("uBaseShaded"), 1);
			for (const tile of tiles) {
				const [left, right] = [column(tile.x), column(tile.x + tile.span)];
				const [top, bottom] = [row(tile.y), row(tile.y + tile.span)];
				gl.scissor(left, canvas.height - bottom, right - left, bottom - top);
				gl.bindTexture(gl.TEXTURE_2D, tile.texture);
				gl.uniform2f(uniform("uBaseOrigin"), tile.x, tile.y);
				gl.uniform2f(uniform("uBaseSize"), tile.span, tile.span);
				gl.drawArrays(gl.TRIANGLES, 0, 3);
			}
			gl.disable(gl.SCISSOR_TEST);
		},
	};
}

// ─── Terrain detail ──────────────────────────────────────────────────────────
//
// Zoomed out, the map shows one image of the whole world. Zoomed in, it lays
// sharper tiles over it, rendered on demand by the game's terrain generator
// running in workers (tile-worker.js). Tiles come in levels: a level-0 tile
// spans 32 chunks and each level halves that, down to one chunk per tile.

const TILE_PIXELS = 256;
const TILE_LEVEL_0_SPAN_CHUNKS = 32;
const TILE_MAX_LEVEL = 5;
/// The whole-world image holds this many pixels per chunk.
const BASE_IMAGE_PIXELS_PER_CHUNK = 4;
/// An image may be stretched by this much before a sharper one is wanted.
const TILE_MAX_STRETCH = 1.5;
/// A coarser level is rendered first as a quick stand-in for the wanted one.
const TILE_PREVIEW_LEVELS_UP = 2;
const TILE_CACHE_SIZE = 320;
const TILE_WORKER_COUNT = Math.min(
	4,
	Math.max(1, (navigator.hardwareConcurrency ?? 4) - 2),
);
const TERRAIN_WASM_URL = "/assets/terrain.wasm";
const MACRO_PACK_FILE = "world.mgmacro";

const terrainTiles = {
	// key -> { texture, used }
	cache: new Map(),
	// Keys being rendered now.
	pending: new Set(),
	// Tiles to render next, most wanted first.
	wanted: [],
	workers: [],
	clock: 0,
};

const tileSpan = (level) => TILE_LEVEL_0_SPAN_CHUNKS / 2 ** level;

// The level whose tiles are sharp enough for the view, or -1 if the
// whole-world image is.
function tileLevelForView() {
	const needed = pixelsPerChunk() / TILE_MAX_STRETCH;
	if (needed <= BASE_IMAGE_PIXELS_PER_CHUNK) return -1;
	const level0 = TILE_PIXELS / TILE_LEVEL_0_SPAN_CHUNKS;
	const level = Math.ceil(Math.log2(needed / level0));
	return Math.max(0, Math.min(TILE_MAX_LEVEL, level));
}

// The tiles of a level that are in view. `x` is where the tile is drawn and
// may lie off the map's ends; `worldX` is the same place on the map.
function tilesInView(level) {
	const { chunks_wide: wide, chunks_high: high } = worldMap.meta;
	const span = tileSpan(level);
	const halfWidth = canvas.width / pixelsPerChunk() / 2;
	const halfHeight = canvas.height / pixelsPerChunk() / 2;
	const columns = wide / span;
	const firstRow = Math.max(0, Math.floor((view.y - halfHeight) / span));
	const lastRow = Math.min(
		high / span - 1,
		Math.floor((view.y + halfHeight) / span),
	);
	const tiles = [];
	for (let row = firstRow; row <= lastRow; row++) {
		const firstColumn = Math.floor((view.x - halfWidth) / span);
		const lastColumn = Math.floor((view.x + halfWidth) / span);
		for (let column = firstColumn; column <= lastColumn; column++) {
			const worldColumn = ((column % columns) + columns) % columns;
			tiles.push({
				key: `${level}/${worldColumn}/${row}`,
				x: column * span,
				worldX: worldColumn * span,
				y: row * span,
				span,
			});
		}
	}
	return tiles;
}

function setMapStatus(text) {
	document.getElementById("mapStatus").textContent = text;
}

// Hand waiting tiles to idle workers.
function dispatchTiles() {
	for (const entry of terrainTiles.workers) {
		if (!entry.ready || entry.busy) continue;
		const tile = terrainTiles.wanted.find(
			({ key }) =>
				!terrainTiles.pending.has(key) && !terrainTiles.cache.has(key),
		);
		if (!tile) return;
		terrainTiles.pending.add(tile.key);
		entry.busy = true;
		entry.worker.postMessage({
			key: tile.key,
			x: tile.worldX,
			y: tile.y,
			span: tile.span,
			pixels: TILE_PIXELS,
		});
	}
}

// Drop the tiles that have gone longest without being drawn.
function trimTileCache() {
	const excess = terrainTiles.cache.size - TILE_CACHE_SIZE;
	if (excess <= 0) return;
	const oldestFirst = [...terrainTiles.cache.entries()].sort(
		(a, b) => a[1].used - b[1].used,
	);
	for (const [key, tile] of oldestFirst.slice(0, excess)) {
		renderer.deleteTexture(tile.texture);
		terrainTiles.cache.delete(key);
	}
}

// Workers start the first time detail is wanted, so a visitor who never
// zooms in never downloads the generator or the macro pack.
function startTileWorkers() {
	if (terrainTiles.workers.length > 0) return;
	setMapStatus("Loading terrain detail…");
	for (let index = 0; index < TILE_WORKER_COUNT; index++) {
		const entry = {
			worker: new Worker("/assets/tile-worker.js"),
			ready: false,
			busy: false,
		};
		entry.worker.addEventListener("message", ({ data }) => {
			if (data.type === "ready") {
				entry.ready = true;
				setMapStatus("");
			} else if (data.type === "failed") {
				setMapStatus(
					`Terrain detail is unavailable (${data.reason}). Build it with: cargo build -p mg_web --release --target wasm32-unknown-unknown, then rebuild the site.`,
				);
				return;
			} else {
				entry.busy = false;
				terrainTiles.pending.delete(data.key);
				terrainTiles.cache.set(data.key, {
					texture: renderer.tileTexture(data.pixels, TILE_PIXELS),
					used: terrainTiles.clock,
				});
				trimTileCache();
				draw();
			}
			dispatchTiles();
		});
		entry.worker.postMessage({
			type: "load",
			wasmUrl: new URL(TERRAIN_WASM_URL, location.href).href,
			packUrl: new URL(MACRO_PACK_FILE, location.href).href,
		});
		terrainTiles.workers.push(entry);
	}
}

// The tiles to draw this frame, coarse first so sharper ones cover them, and
// a request for those that are missing.
function terrainTilesForView() {
	const level = rawLayer ? -1 : tileLevelForView();
	if (level < 0) {
		terrainTiles.wanted = [];
		return [];
	}
	startTileWorkers();
	terrainTiles.clock += 1;
	const ready = [];
	const missing = [];
	const previewLevel = Math.max(0, level - TILE_PREVIEW_LEVELS_UP);
	for (let drawn = 0; drawn <= level; drawn++) {
		for (const tile of tilesInView(drawn)) {
			const cached = terrainTiles.cache.get(tile.key);
			if (cached) {
				cached.used = terrainTiles.clock;
				ready.push({ ...tile, texture: cached.texture });
			} else if (drawn === level || drawn === previewLevel) {
				missing.push({ ...tile, level: drawn });
			}
		}
	}
	// The quick stand-in first, then the wanted level from the middle outwards.
	const fromCentre = (tile) =>
		Math.hypot(
			tile.x + tile.span / 2 - view.x,
			tile.y + tile.span / 2 - view.y,
		);
	terrainTiles.wanted = missing.sort(
		(a, b) => a.level - b.level || fromCentre(a) - fromCentre(b),
	);
	dispatchTiles();
	return ready;
}

// ─── Lines, markers and names (2D canvas over the map) ───────────────────────
//
// What is drawn depends on how far in the view is: `detail` is CSS pixels per
// chunk, about 1.5 with the whole world in view and 200 fully zoomed in.

const detail = () => pixelsPerChunk() / window.devicePixelRatio;

// Indexed like `settlement_sizes` in map.json. `from` is the detail at which
// a size starts to show and `nameFrom` the detail at which it is named;
// `radius` is in CSS pixels.
const SETTLEMENT_STYLES = [
	{ from: 0, nameFrom: 6, radius: 3.5, rgb: [255, 255, 255], square: true },
	{ from: 2.5, nameFrom: 8, radius: 3, rgb: [255, 214, 120], square: true },
	{ from: 5, nameFrom: 11, radius: 2.5, rgb: [240, 170, 110] },
	{ from: 9, nameFrom: 15, radius: 2, rgb: [225, 215, 235] },
	{ from: 14, nameFrom: 20, radius: 1.5, rgb: [150, 140, 175] },
	{ from: 14, nameFrom: 20, radius: 1.5, rgb: [170, 70, 80] },
];
/// Settlement names are this many CSS pixels high, largest size first.
const SETTLEMENT_NAME_SIZES = [13, 12, 11, 10, 10, 10];
// Indexed like `road_kinds` in network.json. `width` is in CSS pixels.
const ROAD_STYLES = [
	{ from: 0, width: 1.5, rgb: [255, 255, 255] },
	{ from: 3, width: 1.2, rgb: [240, 200, 144] },
	{ from: 6, width: 1, rgb: [70, 58, 96], dash: [4, 3] },
];
const RIVER_RGB = [80, 130, 180];
const RIVER_BANK_RGB = [30, 52, 84];
/// A river is drawn at least this many CSS pixels wide, so it shows from afar.
const RIVER_MIN_WIDTH_PX = 1;
/// State names fade out across this range of detail, as settlement names
/// take over.
const STATE_NAMES_FADE_DETAIL = [9, 14];
const LABEL_HALO = "rgba(16, 14, 38, 0.8)";

const cssColour = (rgb, alpha = 1) => `rgba(${rgb.join(",")}, ${alpha})`;

// A line of points as [x, y, ...extra] with `stride` numbers per point, made
// continuous across the east-west seam, with its bounding box.
function unwrappedLine(numbers, stride, offset) {
	const wide = worldMap.meta.chunks_wide;
	const points = [];
	let shift = 0;
	for (let index = 0; index < numbers.length; index += stride) {
		const x = numbers[index] + offset;
		const previous = points.at(-1);
		if (previous) {
			const step = x + shift - previous[0];
			if (step > wide / 2) shift -= wide;
			else if (step < -wide / 2) shift += wide;
		}
		points.push([x + shift, numbers[index + 1] + offset, numbers[index + 2]]);
	}
	const xs = points.map((point) => point[0]);
	const ys = points.map((point) => point[1]);
	return {
		points,
		box: [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)],
	};
}

// Turn network.json into lines ready to draw.
function prepareNetwork(network) {
	const { settlements } = worldMap.meta;
	return {
		// Roads run through the centres of the cells they cross.
		roads: network.roads.map((road) => ({
			kind: road[0],
			...unwrappedLine(road.slice(1), 2, 0.5),
		})),
		rivers: network.rivers.map((river) => unwrappedLine(river, 3, 0)),
		trade: network.trade.map(([from, to, value]) => ({
			value,
			...unwrappedLine(
				[...settlements[from].slice(0, 2), ...settlements[to].slice(0, 2)],
				2,
				0.5,
			),
		})),
	};
}

// Calls `drawAt(toScreen)` once for each place a line is on screen: the map
// repeats east to west, so that can be more than once. `toScreen` maps a
// point in chunks to canvas pixels.
function forEachVisibleLap(box, drawAt) {
	const wide = worldMap.meta.chunks_wide;
	const scale = pixelsPerChunk();
	const halfWidth = canvas.width / scale / 2;
	const halfHeight = canvas.height / scale / 2;
	if (box[3] < view.y - halfHeight || box[1] > view.y + halfHeight) return;
	for (const lap of [-2, -1, 0, 1, 2]) {
		const shift = lap * wide;
		if (
			box[2] + shift < view.x - halfWidth ||
			box[0] + shift > view.x + halfWidth
		)
			continue;
		drawAt(([x, y]) => [
			(x + shift - view.x) * scale + canvas.width / 2,
			(y - view.y) * scale + canvas.height / 2,
		]);
	}
}

// Trace a line through `points`, rounding its corners.
function tracePath(points, toScreen) {
	const screen = points.map(toScreen);
	labelContext.beginPath();
	labelContext.moveTo(screen[0][0], screen[0][1]);
	for (let index = 1; index < screen.length - 1; index++) {
		const [x, y] = screen[index];
		const [nextX, nextY] = screen[index + 1];
		labelContext.quadraticCurveTo(x, y, (x + nextX) / 2, (y + nextY) / 2);
	}
	const last = screen.at(-1);
	labelContext.lineTo(last[0], last[1]);
}

function drawRivers() {
	const ratio = window.devicePixelRatio;
	labelContext.lineCap = "round";
	labelContext.setLineDash([]);
	// Banks first, as a darker line a little wider than the water, then the
	// water over them. A river is never drawn thinner than a hairline.
	const passes = [
		{ colour: cssColour(RIVER_BANK_RGB, 0.9), extra: 2 * ratio },
		{ colour: cssColour(RIVER_RGB), extra: 0 },
	];
	for (const { colour, extra } of passes) {
		labelContext.strokeStyle = colour;
		for (const river of worldMap.network.rivers) {
			forEachVisibleLap(river.box, (toScreen) => {
				// One stroke per stretch, as wide as the river is there.
				for (let index = 0; index + 1 < river.points.length; index++) {
					const [from, to] = [river.points[index], river.points[index + 1]];
					const [fromX, fromY] = toScreen(from);
					const [toX, toY] = toScreen(to);
					const water = Math.max(
						RIVER_MIN_WIDTH_PX * ratio,
						(from[2] + to[2]) * pixelsPerChunk(),
					);
					labelContext.lineWidth = water + extra;
					labelContext.beginPath();
					labelContext.moveTo(fromX, fromY);
					labelContext.lineTo(toX, toY);
					labelContext.stroke();
				}
			});
		}
	}
}

// Lesser roads first, so highways lie on top.
function drawRoads() {
	const ratio = window.devicePixelRatio;
	labelContext.lineCap = "round";
	labelContext.lineJoin = "round";
	for (let kind = ROAD_STYLES.length - 1; kind >= 0; kind--) {
		const style = ROAD_STYLES[kind];
		if (detail() < style.from) continue;
		labelContext.strokeStyle = cssColour(style.rgb, 0.9);
		labelContext.lineWidth = style.width * ratio;
		labelContext.setLineDash(
			(style.dash ?? []).map((length) => length * ratio),
		);
		for (const road of worldMap.network.roads) {
			if (road.kind !== kind) continue;
			forEachVisibleLap(road.box, (toScreen) => {
				tracePath(road.points, toScreen);
				labelContext.stroke();
			});
		}
	}
	labelContext.setLineDash([]);
}

// Straight lines from each settlement to its market. Schematic: a line may
// cross water that the road does not.
function drawTrade() {
	labelContext.lineWidth = window.devicePixelRatio;
	labelContext.setLineDash([]);
	for (const flow of worldMap.network.trade) {
		labelContext.strokeStyle = cssColour(
			rampColour(flow.value).slice(0, 3),
			0.85,
		);
		forEachVisibleLap(flow.box, (toScreen) => {
			tracePath(flow.points, toScreen);
			labelContext.stroke();
		});
	}
}

// Canvas x positions at which chunk column `chunkX` is visible: the map
// repeats east to west, so it can appear more than once.
function screenColumns(chunkX) {
	const wide = worldMap.meta.chunks_wide;
	const scale = pixelsPerChunk();
	const columns = [];
	for (const lap of [-1, 0, 1]) {
		const x = (chunkX + lap * wide - view.x) * scale + canvas.width / 2;
		if (x > -20 && x < canvas.width + 20) columns.push(x);
	}
	return columns;
}

const screenRow = (chunkY) =>
	(chunkY - view.y) * pixelsPerChunk() + canvas.height / 2;

// Text with a dark halo, so it reads over any map colour.
function drawLabel(text, x, y, size, alpha = 1) {
	labelContext.globalAlpha = alpha;
	labelContext.font = `600 ${size}px "Public Sans", system-ui, sans-serif`;
	labelContext.lineJoin = "round";
	labelContext.lineWidth = size / 4;
	labelContext.strokeStyle = LABEL_HALO;
	labelContext.strokeText(text, x, y);
	labelContext.fillStyle = "#fff";
	labelContext.fillText(text, x, y);
	labelContext.globalAlpha = 1;
}

// Whether two boxes [left, top, right, bottom] overlap.
const boxesOverlap = (a, b) =>
	a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1];

// Settlements as markers, smaller sizes appearing as the view closes in, and
// then their names. A name that would overlap a larger settlement's name is
// left out.
function drawSettlements() {
	const ratio = window.devicePixelRatio;
	const { settlements } = worldMap.meta;
	const names = [];
	// Largest last, so they lie on top.
	for (let size = SETTLEMENT_STYLES.length - 1; size >= 0; size--) {
		const style = SETTLEMENT_STYLES[size];
		if (detail() < style.from) continue;
		const radius = style.radius * ratio;
		labelContext.fillStyle = cssColour(style.rgb);
		labelContext.strokeStyle = cssColour([16, 14, 38]);
		labelContext.lineWidth = ratio;
		for (const [chunkX, chunkY, settlementSize, , name] of settlements) {
			if (settlementSize !== size) continue;
			const y = screenRow(chunkY + 0.5);
			if (y < -20 || y > canvas.height + 20) continue;
			for (const x of screenColumns(chunkX + 0.5)) {
				labelContext.beginPath();
				if (style.square) {
					labelContext.rect(x - radius, y - radius, radius * 2, radius * 2);
				} else {
					labelContext.arc(x, y, radius, 0, Math.PI * 2);
				}
				labelContext.fill();
				labelContext.stroke();
				if (detail() >= style.nameFrom) {
					names.push({ name, size, x: x + radius + 4 * ratio, y });
				}
			}
		}
	}

	labelContext.textAlign = "left";
	labelContext.textBaseline = "middle";
	const placed = [];
	for (const { name, size, x, y } of names.reverse()) {
		const height = SETTLEMENT_NAME_SIZES[size] * ratio;
		labelContext.font = `600 ${height}px "Public Sans", system-ui, sans-serif`;
		const width = labelContext.measureText(name).width;
		const box = [x, y - height * 0.6, x + width, y + height * 0.6];
		if (placed.some((other) => boxesOverlap(box, other))) continue;
		placed.push(box);
		drawLabel(name, x, y, height);
	}
}

// State names over their territory, larger for larger states. A name that
// would overlap one already placed is left out.
function drawStateNames() {
	const [fadeFrom, fadeTo] = STATE_NAMES_FADE_DETAIL;
	const alpha = 1 - (detail() - fadeFrom) / (fadeTo - fadeFrom);
	if (alpha <= 0) return;
	const ratio = window.devicePixelRatio;
	const placed = [];
	// The lore's states first, then by size.
	const states = worldMap.meta.factions
		.map((faction, index) => ({ faction, at: worldMap.labelPositions[index] }))
		.sort(
			(a, b) =>
				b.faction.authored - a.faction.authored ||
				b.faction.area_chunks - a.faction.area_chunks,
		);
	labelContext.textAlign = "center";
	labelContext.textBaseline = "middle";
	for (const { faction, at } of states) {
		const span = Math.sqrt(faction.area_chunks) * pixelsPerChunk();
		const size = Math.max(11 * ratio, Math.min(30 * ratio, span * 0.2));
		labelContext.font = `600 ${size}px "Public Sans", system-ui, sans-serif`;
		const halfWidth = labelContext.measureText(faction.name).width / 2;
		const y = screenRow(at.y);
		for (const x of screenColumns(at.x)) {
			const box = [
				x - halfWidth,
				y - size * 0.6,
				x + halfWidth,
				y + size * 0.6,
			];
			if (placed.some((other) => boxesOverlap(box, other))) continue;
			placed.push(box);
			drawLabel(faction.name, x, y, size, Math.min(1, alpha));
		}
	}
}

function drawSpawnPin() {
	const ratio = window.devicePixelRatio;
	const y = screenRow(spawn.y + 0.5);
	for (const x of screenColumns(spawn.x + 0.5)) {
		labelContext.beginPath();
		labelContext.arc(x, y, 7 * ratio, 0, Math.PI * 2);
		labelContext.lineWidth = 5 * ratio;
		labelContext.strokeStyle = "#000";
		labelContext.stroke();
		labelContext.lineWidth = 2.5 * ratio;
		labelContext.strokeStyle = "#fff";
		labelContext.stroke();
	}
}

// What can be switched on over the map, bottom layer first.
const MAP_FEATURES = [
	{ name: "Rivers", draw: drawRivers, on: true },
	{ name: "Roads", draw: drawRoads, on: true },
	{ name: "Trade", draw: drawTrade, on: false },
	{ name: "Settlements", draw: drawSettlements, on: true },
	{ name: "State names", draw: drawStateNames, on: true },
];
const shown = new Set(
	MAP_FEATURES.filter((feature) => feature.on).map((feature) => feature.name),
);

let drawQueued = false;
// Redraw on the next frame; many calls in one frame draw once.
function draw() {
	if (!renderer || drawQueued) return;
	drawQueued = true;
	requestAnimationFrame(() => {
		drawQueued = false;
		renderer.draw({
			baseImage: rawLayer ?? TERRAIN_IMAGE,
			showProvinces: !rawLayer,
			stateBands: mapMode.stateBands === true,
			tiles: terrainTilesForView(),
		});
		labelContext.clearRect(0, 0, labelCanvas.width, labelCanvas.height);
		// A raw layer is shown bare.
		if (!rawLayer) {
			for (const feature of MAP_FEATURES) {
				if (shown.has(feature.name)) feature.draw();
			}
		}
		drawSpawnPin();
	});
}

// ─── Controls ────────────────────────────────────────────────────────────────

function markChecked() {
	for (const button of document.querySelectorAll("#mapModes button")) {
		button.setAttribute(
			"aria-checked",
			String(!rawLayer && button.dataset.mode === mapMode.id),
		);
	}
	for (const button of document.querySelectorAll("#layers button")) {
		button.setAttribute(
			"aria-checked",
			String(button.dataset.layer === rawLayer),
		);
	}
}

function showMapMode(mode) {
	mapMode = mode;
	rawLayer = null;
	renderer.setProvinceColours(mode.colour);
	markChecked();
	renderLegend();
	draw();
}

function showRawLayer(fileName) {
	rawLayer = fileName;
	markChecked();
	renderLegend();
	draw();
}

function radioButton(text, onClick) {
	const button = document.createElement("button");
	button.type = "button";
	button.role = "radio";
	button.textContent = text;
	button.addEventListener("click", onClick);
	return button;
}

function renderMapModeButtons() {
	const row = document.getElementById("mapModes");
	for (const mode of MAP_MODES) {
		const button = radioButton(mode.name, () => showMapMode(mode));
		button.dataset.mode = mode.id;
		row.append(button);
	}
}

// One labelled row of raw-layer buttons per group in map.json.
function renderRawLayerButtons() {
	const container = document.getElementById("layers");
	for (const group of worldMap.meta.layer_groups ?? []) {
		const row = document.createElement("div");
		row.className = "layer-group";
		const name = document.createElement("span");
		name.className = "layer-group-name";
		name.textContent = group.name;
		row.append(name);
		for (const fileName of group.layers) {
			if (fileName === TERRAIN_IMAGE) continue;
			const button = radioButton(layerLabel(fileName), () =>
				showRawLayer(fileName),
			);
			button.dataset.layer = fileName;
			row.append(button);
		}
		if (row.childElementCount > 1) container.append(row);
	}
}

function renderShowToggles() {
	const container = document.getElementById("overlays");
	// Listed top layer first.
	for (const { name } of MAP_FEATURES.toReversed()) {
		const label = document.createElement("label");
		const checkbox = document.createElement("input");
		checkbox.type = "checkbox";
		checkbox.checked = shown.has(name);
		checkbox.addEventListener("change", () => {
			if (checkbox.checked) shown.add(name);
			else shown.delete(name);
			renderLegend();
			draw();
		});
		label.append(checkbox, ` ${name}`);
		container.append(label);
	}
}

function moveSpawn(x, y) {
	spawn.x = wrapColumn(x);
	spawn.y = Math.max(0, Math.min(worldMap.meta.chunks_high - 1, Math.floor(y)));
	renderSpawnReadout();
	draw();
}

function selectProvince(provinceId) {
	selectedProvince = provinceId;
	renderProvincePanel();
	draw();
}

function hoverProvince(provinceId) {
	if (provinceId === hoveredProvince) return;
	hoveredProvince = provinceId;
	renderHoverReadout();
	draw();
}

// A press that does not move is a click (select the province and set the
// spawn point); one that moves pans the map.
let press = null;
labelCanvas.addEventListener("pointerdown", (event) => {
	if (!renderer) return;
	labelCanvas.setPointerCapture(event.pointerId);
	press = { x: event.offsetX, y: event.offsetY, dragged: false };
});
labelCanvas.addEventListener("pointermove", (event) => {
	if (!renderer) return;
	if (!press) {
		hoverProvince(provinceUnder(event.offsetX, event.offsetY));
		return;
	}
	const [dx, dy] = [event.offsetX - press.x, event.offsetY - press.y];
	if (!press.dragged && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
	press.dragged = true;
	mapFrame.classList.add("panning");
	const chunksPerCssPixel =
		canvas.width / canvas.clientWidth / pixelsPerChunk();
	view.x -= dx * chunksPerCssPixel;
	view.y -= dy * chunksPerCssPixel;
	press.x = event.offsetX;
	press.y = event.offsetY;
	constrainView();
	draw();
});
labelCanvas.addEventListener("pointerup", (event) => {
	if (press && !press.dragged) {
		const chunk = chunkUnder(event.offsetX, event.offsetY);
		moveSpawn(chunk.x, chunk.y);
		selectProvince(provinceUnder(event.offsetX, event.offsetY));
	}
	press = null;
	mapFrame.classList.remove("panning");
});
labelCanvas.addEventListener("pointercancel", () => {
	press = null;
	mapFrame.classList.remove("panning");
});
labelCanvas.addEventListener("pointerleave", () => hoverProvince(0));
// Scrolling over the map zooms it, towards the pointer.
labelCanvas.addEventListener(
	"wheel",
	(event) => {
		if (!renderer) return;
		event.preventDefault();
		// A pinch arrives as a wheel event with ctrlKey set and small deltas.
		const rate = event.ctrlKey ? WHEEL_ZOOM_RATE * 6 : WHEEL_ZOOM_RATE;
		zoomAt(Math.exp(-event.deltaY * rate), event.offsetX, event.offsetY);
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
	if (!renderer) return;
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
		selectProvince(chunkAt(spawn.x, spawn.y).provinceId);
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
	const status = document.getElementById("mapStatus");
	try {
		const [meta, chunks, network] = await Promise.all([
			fetch("map.json").then((response) => response.json()),
			fetch("chunks.bin").then((response) => response.arrayBuffer()),
			fetch("network.json").then((response) => response.json()),
		]);
		worldMap = { meta, chunks: new Uint8Array(chunks) };
		worldMap.network = prepareNetwork(network);
		worldMap.sea = await imagePixels(SEA_IMAGE);
		worldMap.roadKinds = network.road_kinds;
	} catch {
		status.textContent =
			"Map data not found. Export it with: margins_grip export site-map site/dist/map";
		return;
	}
	worldMap.labelPositions = stateLabelPositions();
	Object.assign(view, {
		x: worldMap.meta.chunks_wide / 2,
		y: worldMap.meta.chunks_high / 2,
	});
	resizeCanvas();
	renderer = createRenderer();
	if (!renderer) {
		status.textContent =
			"The map needs WebGL 2, which this browser does not provide.";
		return;
	}
	renderMapModeButtons();
	renderRawLayerButtons();
	renderShowToggles();
	showMapMode(mapMode);
	renderSpawnReadout();
	selectProvince(chunkAt(spawn.x, spawn.y).provinceId);
}

window.addEventListener("resize", resizeCanvas);
loadWorldMap();
