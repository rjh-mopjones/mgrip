// Erosion sandbox page: sliders for the landscape step of spec 013, run on
// the real world in a worker (sandbox-worker.js).

const TERRAIN_WASM_URL = "/assets/terrain.wasm";
const MACRO_PACK_URL = "/map/world.mgmacro";
/// Steps run between one drawing and the next.
const STEPS_PER_FRAME = 4;
/// The run stops here; "Play" carries on for as many again.
const STEPS_PER_RUN = 600;

// Each slider: the setting it controls, its range, and where it starts.
const SLIDERS = [
	{
		key: "erodibility",
		label: "Erodibility",
		min: 0.002,
		max: 0.08,
		step: 0.001,
		value: 0.021,
		digits: 3,
		note: "How easily rivers cut rock. Higher gives lower, broader land.",
	},
	{
		key: "uplift",
		label: "Uplift rate",
		min: 0.001,
		max: 0.02,
		step: 0.0005,
		value: 0.008,
		digits: 4,
		note: "How fast the land rises. Higher gives taller land.",
	},
	{
		key: "upliftLimit",
		label: "Uplift stops at",
		min: 0.2,
		max: 2,
		step: 0.05,
		value: 1,
		digits: 2,
		note: "Height at which land stops rising, in units of 200 blocks. Bounds land where little rain falls.",
	},
	{
		key: "flowExponent",
		label: "River strength",
		min: 0.3,
		max: 0.7,
		step: 0.01,
		value: 0.46,
		digits: 2,
		note: "How much more a big river cuts than a small one. Higher gives deeper main valleys.",
	},
	{
		key: "slopeCreep",
		label: "Slope creep",
		min: 0,
		max: 0.15,
		step: 0.005,
		value: 0.08,
		digits: 3,
		note: "How fast slopes round off. Higher gives smoother ridges.",
	},
	{
		key: "interiorUplift",
		label: "Uplift: interiors",
		min: 0,
		max: 1,
		step: 0.05,
		value: 0.55,
		digits: 2,
		note: "Whole continents rising, most in the middle.",
	},
	{
		key: "rangeUplift",
		label: "Uplift: mountain belts",
		min: 0,
		max: 1.5,
		step: 0.05,
		value: 0.75,
		digits: 2,
		note: "Rising along mountain belts.",
	},
	{
		key: "faultUplift",
		label: "Uplift: faults",
		min: 0,
		max: 1.5,
		step: 0.05,
		value: 0.7,
		digits: 2,
		note: "Rising along plate boundaries.",
	},
	{
		key: "iceWidening",
		label: "Ice widens valleys",
		min: 0,
		max: 0.6,
		step: 0.01,
		value: 0.25,
		digits: 2,
		note: "How hard ice streams on the night side pull the ground beside them down. Higher gives broader troughs.",
	},
	{
		key: "nightRunoff",
		label: "Night-side run-off",
		min: 0,
		max: 1,
		step: 0.01,
		value: 0.07,
		digits: 2,
		note: "Share of rain that still runs off on the frozen side.",
	},
	{
		key: "dayRunoff",
		label: "Day-side run-off",
		min: 0,
		max: 1,
		step: 0.01,
		value: 0.02,
		digits: 2,
		note: "Share of rain that still runs off on the evaporating side.",
	},
	{
		key: "riverThreshold",
		label: "River shows from",
		min: 5,
		max: 300,
		step: 1,
		value: 90,
		digits: 0,
		note: "Gathered run-off at which a cell is drawn as river. Does not change the land.",
	},
];
const VIEWS = ["Terrain", "Uplift", "Run-off"];

const settings = { view: 0 };
for (const slider of SLIDERS) settings[slider.key] = slider.value;

const canvas = document.getElementById("sandboxCanvas");
const context = canvas.getContext("2d");
const status = document.getElementById("sandboxStatus");
const playButton = document.getElementById("play");

const worker = new Worker("/assets/sandbox-worker.js");
let playing = true;
let ready = false;
// Whether the worker is working on a request; one is sent at a time.
let busy = false;
// Set when a setting changes while the worker is busy.
let redrawWanted = false;
let stepsThisRun = 0;

function showSettings() {
	const lines = SLIDERS.map(
		({ key, digits }) => `${key}: ${settings[key].toFixed(digits)}`,
	);
	document.getElementById("settingsText").textContent = lines.join("\n");
}

// Ask the worker for the next frame: more steps if playing, else a redraw.
function request({ reset = false } = {}) {
	if (!ready || busy) {
		redrawWanted = true;
		return;
	}
	busy = true;
	redrawWanted = false;
	const steps = playing && !reset ? STEPS_PER_FRAME : 0;
	stepsThisRun += steps;
	worker.postMessage({ type: reset ? "reset" : "run", steps, settings });
}

function setPlaying(next) {
	playing = next;
	playButton.textContent = playing ? "Pause" : "Play";
	if (playing) {
		stepsThisRun = 0;
		request();
	}
}

function showFrame(frame) {
	if (canvas.width !== frame.width) {
		canvas.width = frame.width;
		canvas.height = frame.width / 2;
	}
	context.putImageData(
		new ImageData(frame.pixels, frame.width, frame.width / 2),
		0,
		0,
	);
	const set = (id, text) => {
		document.getElementById(id).textContent = text;
	};
	set("statSteps", frame.steps);
	set("statPeak", `${Math.round(frame.peakBlocks)} blocks`);
	set("statChange", `${frame.changeBlocks.toFixed(3)} blocks`);
	set("statRivers", frame.riverCells.toLocaleString());
	set("statLakes", frame.lakeCells.toLocaleString());
}

worker.addEventListener("message", ({ data }) => {
	if (data.type === "failed") {
		status.textContent = `The sandbox could not start (${data.reason}). It needs the terrain generator (cargo build -p mg_web --release --target wasm32-unknown-unknown, then rebuild the site) and the map export (margins_grip export site-map site/dist/map).`;
		return;
	}
	if (data.type === "ready") {
		ready = true;
		status.textContent = "";
		request();
		return;
	}
	busy = false;
	showFrame(data);
	if (playing && stepsThisRun >= STEPS_PER_RUN) setPlaying(false);
	if (playing || redrawWanted) request();
});

function buildControls() {
	const sliders = document.getElementById("sliders");
	for (const slider of SLIDERS) {
		const label = document.createElement("label");
		label.title = slider.note;
		const name = document.createElement("span");
		name.textContent = slider.label;
		const input = document.createElement("input");
		input.type = "range";
		for (const attribute of ["min", "max", "step", "value"]) {
			input[attribute] = slider[attribute];
		}
		const value = document.createElement("output");
		value.textContent = slider.value.toFixed(slider.digits);
		input.addEventListener("input", () => {
			settings[slider.key] = Number(input.value);
			value.textContent = settings[slider.key].toFixed(slider.digits);
			showSettings();
			request();
		});
		label.append(name, input, value);
		sliders.append(label);
	}

	const views = document.getElementById("views");
	VIEWS.forEach((name, index) => {
		const button = document.createElement("button");
		button.type = "button";
		button.role = "radio";
		button.textContent = name;
		button.setAttribute("aria-checked", String(index === settings.view));
		button.addEventListener("click", () => {
			settings.view = index;
			for (const other of views.querySelectorAll("button")) {
				other.setAttribute("aria-checked", String(other === button));
			}
			request();
		});
		views.append(button);
	});
}

playButton.addEventListener("click", () => setPlaying(!playing));
document.getElementById("reset").addEventListener("click", () => {
	stepsThisRun = 0;
	request({ reset: true });
	if (!playing) setPlaying(true);
});

buildControls();
showSettings();
worker.postMessage({
	type: "load",
	wasmUrl: new URL(TERRAIN_WASM_URL, location.href).href,
	packUrl: new URL(MACRO_PACK_URL, location.href).href,
});
