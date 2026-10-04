// Map page: a campaign-style map of the world. Provinces can be hovered and
// selected, map modes recolour them, and a spawn chunk can be picked to
// launch the web build there.
//
// Map data is exported by the Rust CLI (`margins_grip export site-map`) into
// this page's directory: one PNG per generation layer, transparent overlay
// PNGs, chunks.bin with one record per chunk, and map.json describing them.
//
// Drawing follows the usual campaign-map scheme: one texture holds the
// province id of every chunk, another holds one colour per province. A map
// mode, a hover or a selection only rewrites the small colour table.

const DEFAULT_SPAWN_CHUNK = { x: 440, y: 220 };
const TERRAIN_IMAGE = "macromap.png";
const MAX_ZOOM = 16;
const ZOOM_STEP = 1.5;
const WHEEL_ZOOM_RATE = 0.0015;
/// A press that moves further than this many pixels is a drag, not a click.
const DRAG_THRESHOLD_PX = 4;
/// The shader has this many overlay slots.
const MAX_OVERLAYS = 3;
const STATE_NAMES_TOGGLE = "State names";

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
	return [channel(5), channel(3), channel(1), 205];
}

// Each map mode gives one colour per province, laid over the terrain.
const MAP_MODES = [
	{
		id: "political",
		name: "Political",
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
const shown = new Set([STATE_NAMES_TOGGLE]);
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

const provinceById = (provinceId) => worldMap.meta.provinces[provinceId - 1];
const factionById = (factionId) => worldMap.meta.factions[factionId - 1];
const factionName = (factionId) =>
	factionById(factionId).name ?? `Minor state ${factionId}`;

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
	return `${sizes[nearest[2]]}, ${where}`;
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
		["Held by", ownerText(province)],
		["Biome", readable(province.biome)],
		["Area", `${province.area_chunks} chunks`],
		["Habitability", province.habitability.toFixed(2)],
		["Light level", province.light.toFixed(2)],
		["Resources", province.resources.toFixed(2)],
		["Features", features.length ? features.join(", ") : "none"],
		["Settlements", settlementSummary(selectedProvince)],
	];
	document.getElementById("panelTitle").textContent =
		`Province ${selectedProvince}`;
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
	readout.textContent = `Province ${hoveredProvince}, ${owner}`;
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
	if (shown.has("Settlements")) {
		for (const dot of meta.settlement_dots ?? []) {
			if (dot.size !== "Ruins") parts.push(swatch(dot.rgb, dot.size));
		}
	}
	if (shown.has("Roads")) {
		for (const road of meta.road_colours ?? []) {
			parts.push(swatch(road.rgb, road.kind));
		}
	}
	if (shown.has("Trade")) {
		parts.push(
			'<span class="legend-item">Trade: line from a settlement to its market <span class="swatch ramp"></span> brighter is richer</span>',
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
	return chunkAt(wrapColumn(chunk.x), Math.floor(chunk.y)).provinceId;
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
uniform sampler2D uOverlay0;
uniform sampler2D uOverlay1;
uniform sampler2D uOverlay2;
uniform vec3 uOverlayShown;
uniform vec2 uCanvas;        // pixels
uniform vec2 uWorld;         // chunks
uniform vec2 uCentre;        // chunk at the middle of the canvas
uniform float uPixelsPerChunk;
uniform float uPixelRatio;
uniform float uProvinceLayer; // 0 hides tints and borders (raw layers)
uniform int uHovered;
uniform int uSelected;

out vec4 colour;

const vec3 BORDER = vec3(0.063, 0.055, 0.149);
const vec3 OFF_MAP = vec3(0.933, 0.945, 0.957);

int unpackId(vec4 texel) {
	return int(texel.r * 255.0 + 0.5) + int(texel.g * 255.0 + 0.5) * 256;
}

// The map joins east to west.
int provinceAt(ivec2 cell) {
	int wide = int(uWorld.x);
	cell.x = ((cell.x % wide) + wide) % wide;
	cell.y = clamp(cell.y, 0, int(uWorld.y) - 1);
	return unpackId(texelFetch(uProvinceIds, cell, 0));
}

int ownerOf(int province) {
	return unpackId(texelFetch(uProvinces, ivec2(province, 1), 0));
}

// How much of a border of the given half-width (pixels) covers a point that
// is 'distance' pixels from the edge.
float borderCover(float distance, float halfWidth) {
	return clamp(halfWidth + 0.5 - distance, 0.0, 1.0);
}

void main() {
	vec2 pixel = vec2(gl_FragCoord.x, uCanvas.y - gl_FragCoord.y);
	vec2 chunk = uCentre + (pixel - uCanvas * 0.5) / uPixelsPerChunk;
	if (chunk.y < 0.0 || chunk.y >= uWorld.y) {
		colour = vec4(OFF_MAP, 1.0);
		return;
	}
	vec2 uv = chunk / uWorld;
	vec3 shade = texture(uBase, uv).rgb;

	ivec2 cell = ivec2(floor(chunk));
	int province = provinceAt(cell);
	if (province != 0 && uProvinceLayer > 0.0) {
		// Tint the terrain rather than cover it, so relief shows through.
		vec4 tint = texelFetch(uProvinces, ivec2(province, 0), 0);
		float relief = dot(shade, vec3(0.299, 0.587, 0.114));
		shade = mix(shade, tint.rgb * mix(0.7, 1.15, relief), tint.a);
		if (province == uSelected) {
			shade = mix(shade, vec3(1.0), 0.28);
		} else if (province == uHovered) {
			shade = mix(shade, vec3(1.0), 0.16);
		}

		// Distance, in pixels, to the nearest cell edge with another province
		// beyond it, and to the nearest with another state (or the sea).
		vec2 inCell = fract(chunk);
		float toProvince = 1e6;
		float toState = 1e6;
		int owner = ownerOf(province);
		ivec2 steps[4] = ivec2[4](ivec2(-1, 0), ivec2(1, 0), ivec2(0, -1), ivec2(0, 1));
		float edges[4] = float[4](inCell.x, 1.0 - inCell.x, inCell.y, 1.0 - inCell.y);
		for (int side = 0; side < 4; side++) {
			int neighbour = provinceAt(cell + steps[side]);
			if (neighbour == province) continue;
			float distance = edges[side] * uPixelsPerChunk;
			toProvince = min(toProvince, distance);
			if (neighbour == 0 || ownerOf(neighbour) != owner) {
				toState = min(toState, distance);
			}
		}
		// Province borders fade out when zoomed far out; state borders stay.
		float provinceStrength = mix(0.25, 0.7, smoothstep(1.5, 5.0, uPixelsPerChunk / uPixelRatio));
		shade = mix(shade, BORDER, borderCover(toProvince, 0.5 * uPixelRatio) * provinceStrength);
		shade = mix(shade, BORDER, borderCover(toState, 1.0 * uPixelRatio) * 0.9);
		if (province == uSelected) {
			shade = mix(shade, vec3(1.0), borderCover(toProvince, 1.25 * uPixelRatio));
		}
	}

	// Overlay images are premultiplied.
	vec4 overlay0 = texture(uOverlay0, uv) * uOverlayShown.x;
	shade = shade * (1.0 - overlay0.a) + overlay0.rgb;
	vec4 overlay1 = texture(uOverlay1, uv) * uOverlayShown.y;
	shade = shade * (1.0 - overlay1.a) + overlay1.rgb;
	vec4 overlay2 = texture(uOverlay2, uv) * uOverlayShown.z;
	shade = shade * (1.0 - overlay2.a) + overlay2.rgb;

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
	const uniform = (name) => gl.getUniformLocation(program, name);

	const { meta, chunks } = worldMap;
	const [wide, high] = [meta.chunks_wide, meta.chunks_high];
	const provinceCount = meta.provinces.length;

	// Texture units: 0 base image, 1 province ids, 2 province table, 3+ overlays.
	const samplers = ["uBase", "uProvinceIds", "uProvinces"];
	for (let slot = 0; slot < MAX_OVERLAYS; slot++)
		samplers.push(`uOverlay${slot}`);
	samplers.forEach((name, unit) => {
		gl.uniform1i(uniform(name), unit);
	});

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
	const imageTexture = (fileName, premultiplied) => {
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
			gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, premultiplied);
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
		draw({ baseImage, overlayImages, showProvinces }) {
			gl.viewport(0, 0, canvas.width, canvas.height);
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, imageTexture(baseImage, false));
			// Magnified terrain stays sharp: chunks are the unit.
			gl.texParameteri(
				gl.TEXTURE_2D,
				gl.TEXTURE_MAG_FILTER,
				view.zoom < 6 ? gl.LINEAR : gl.NEAREST,
			);
			const overlayShown = [0, 0, 0];
			overlayImages.slice(0, MAX_OVERLAYS).forEach((fileName, slot) => {
				gl.activeTexture(gl.TEXTURE3 + slot);
				gl.bindTexture(gl.TEXTURE_2D, imageTexture(fileName, true));
				overlayShown[slot] = 1;
			});
			gl.uniform3fv(uniform("uOverlayShown"), overlayShown);
			gl.uniform2f(uniform("uCanvas"), canvas.width, canvas.height);
			gl.uniform2f(uniform("uWorld"), wide, high);
			gl.uniform2f(uniform("uCentre"), view.x, view.y);
			gl.uniform1f(uniform("uPixelsPerChunk"), pixelsPerChunk());
			gl.uniform1f(uniform("uPixelRatio"), window.devicePixelRatio);
			gl.uniform1f(uniform("uProvinceLayer"), showProvinces ? 1 : 0);
			gl.uniform1i(uniform("uHovered"), hoveredProvince);
			gl.uniform1i(uniform("uSelected"), selectedProvince);
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		},
	};
}

// ─── Labels and the spawn pin (2D canvas over the map) ───────────────────────

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

// Names of the authored states over their territory, larger for larger
// states. A name that would overlap a larger state's name is left out.
function drawStateNames() {
	const ratio = window.devicePixelRatio;
	const placed = [];
	const states = worldMap.meta.factions
		.map((faction, index) => ({ faction, at: worldMap.labelPositions[index] }))
		.filter(({ faction }) => faction.name)
		.sort((a, b) => b.faction.area_chunks - a.faction.area_chunks);
	labelContext.textAlign = "center";
	labelContext.textBaseline = "middle";
	labelContext.lineJoin = "round";
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
			const overlaps = placed.some(
				(other) =>
					box[0] < other[2] &&
					box[2] > other[0] &&
					box[1] < other[3] &&
					box[3] > other[1],
			);
			if (overlaps) continue;
			placed.push(box);
			labelContext.lineWidth = size / 4;
			labelContext.strokeStyle = "rgba(16, 14, 38, 0.8)";
			labelContext.strokeText(faction.name, x, y);
			labelContext.fillStyle = "#fff";
			labelContext.fillText(faction.name, x, y);
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

let drawQueued = false;
// Redraw on the next frame; many calls in one frame draw once.
function draw() {
	if (!renderer || drawQueued) return;
	drawQueued = true;
	requestAnimationFrame(() => {
		drawQueued = false;
		const overlays = (worldMap.meta.overlays ?? [])
			.filter((overlay) => shown.has(overlay.name))
			.map((overlay) => overlay.file);
		renderer.draw({
			baseImage: rawLayer ?? TERRAIN_IMAGE,
			overlayImages: overlays,
			showProvinces: !rawLayer,
		});
		labelContext.clearRect(0, 0, labelCanvas.width, labelCanvas.height);
		if (!rawLayer && shown.has(STATE_NAMES_TOGGLE)) drawStateNames();
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
	const names = [
		STATE_NAMES_TOGGLE,
		...(worldMap.meta.overlays ?? []).map((overlay) => overlay.name),
	];
	for (const name of names) {
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
		const [meta, chunks] = await Promise.all([
			fetch("map.json").then((response) => response.json()),
			fetch("chunks.bin").then((response) => response.arrayBuffer()),
		]);
		worldMap = { meta, chunks: new Uint8Array(chunks) };
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
