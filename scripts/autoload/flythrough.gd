extends Node

## Automated flythrough for visual testing.
## Run with:
##   godot --flythrough            # scenic landscape shots
##   godot --flythrough-boundary   # seam / chunk-boundary shots
## Screenshots saved to:
##   /tmp/mgrip_flythrough/scene/frame_001.png … frame_NNN.png
##   /tmp/mgrip_flythrough/boundary/frame_001.png … frame_NNN.png
## Exits automatically when done.

const BASE_SCREENSHOT_DIR := "/tmp/mgrip_flythrough"
const MODE_SCENE := "scene"
const MODE_BOUNDARY := "boundary"
const WARMUP_SECS    := 2.5   # time for terrain to generate + player to spawn
const HOLD_SECS      := 1.2   # seconds at each waypoint before screenshot
const EXIT_DELAY     := 1.5   # flush time after last screenshot
const CLEARANCE      := 8.0

## Terrain-aware flythrough shots. Each shot samples a focus point on the surface,
## then places the camera at a nearby vantage so screenshots stay above ground and
## read the landscape instead of the underside of overhangs.
const SCENE_SHOT_SPECS: Array[Dictionary] = [
	{"focus": Vector2(  8,  20), "camera": Vector2(-130,  -46), "height": 22.0},
	{"focus": Vector2( 82,  40), "camera": Vector2( -86,   20), "height": 14.0},
	{"focus": Vector2(-92,  70), "camera": Vector2( -24, -132), "height": 10.0},
	{"focus": Vector2( 42, -86), "camera": Vector2(   2,  -24), "height": 24.0},
	{"focus": Vector2(-52, -46), "camera": Vector2( 118,   36), "height": 12.0},
]

## Boundary-focused shots that intentionally straddle chunk seams and chunk corners.
## These are used to smoke-test visible cracks between neighboring chunks.
const BOUNDARY_SHOT_SPECS: Array[Dictionary] = [
	{"focus": Vector2( 254,  72), "camera": Vector2( 232,  66), "height":  6.0},
	{"focus": Vector2( 258, 116), "camera": Vector2( 224,  88), "height":  7.0},
	{"focus": Vector2( 110, 254), "camera": Vector2(  88, 226), "height":  7.0},
	{"focus": Vector2(-254, -82), "camera": Vector2(-220, -62), "height":  7.0},
	{"focus": Vector2( 252,-108), "camera": Vector2( 214, -82), "height":  8.0},
]

var _active  := false
var _player:  CharacterBody3D
var _head:    Node3D
var _world:   Node3D
var _spawn:   Vector3
var _shots:   Array[Dictionary] = []
var _mode    := ""
var _screenshot_dir := BASE_SCREENSHOT_DIR
var _phase    := "warmup"
var _timer    := 0.0
var _current  := 0

func _ready() -> void:
	_mode = _detect_mode(OS.get_cmdline_args())
	if _mode.is_empty():
		return
	_active = true
	_screenshot_dir = "%s/%s" % [BASE_SCREENSHOT_DIR, _mode]
	DirAccess.make_dir_recursive_absolute(_screenshot_dir)
	print("=== FLYTHROUGH MODE (%s) ===  output: %s" % [_mode, _screenshot_dir])

func _process(delta: float) -> void:
	if not _active:
		return

	# Wait until player exists in the scene tree
	if not _player:
		var p := get_tree().root.find_child("Player", true, false)
		if not p:
			return
		_player = p as CharacterBody3D
		_head   = _player.get_node("Head") as Node3D
		_world  = get_tree().root.find_child("World", true, false) as Node3D
		_player.set_physics_process(false)
		_player.set_process_unhandled_input(false)
		return

	_timer += delta

	match _phase:
		"warmup":
			if _timer >= WARMUP_SECS:
				_spawn  = _player.position
				_shots = _build_shots()
				if _shots.is_empty():
					push_error("Flythrough: failed to build any valid shots")
					get_tree().quit()
					return
				_phase  = "holding"
				_timer  = 0.0
				_apply_shot(_current)

		"holding":
			_apply_shot(_current)
			if _timer >= HOLD_SECS:
				_phase = "capturing"
				_timer = 0.0
				_capture(_current)

		"capturing":
			_apply_shot(_current)

		"exiting":
			if _timer >= EXIT_DELAY:
				print("=== FLYTHROUGH DONE: %d frames saved to %s ===" \
					% [_shots.size(), _screenshot_dir])
				get_tree().quit()

func _build_shots() -> Array[Dictionary]:
	var shots: Array[Dictionary] = []
	for spec in _shot_specs_for_mode():
		var focus := _land_point(spec["focus"])
		var camera := _land_point(spec["camera"])
		if focus == Vector3.ZERO or camera == Vector3.ZERO:
			continue

		camera.y += spec["height"] + CLEARANCE
		focus.y += CLEARANCE * 0.5

		var look := (focus - camera).normalized()
		var yaw := rad_to_deg(atan2(-look.x, -look.z))
		var pitch := rad_to_deg(asin(clampf(look.y, -1.0, 1.0)))

		shots.append({
			"camera": camera,
			"yaw": yaw,
			"pitch": pitch,
		})

	return shots

func _shot_specs_for_mode() -> Array[Dictionary]:
	if _mode == MODE_BOUNDARY:
		return BOUNDARY_SHOT_SPECS
	return SCENE_SHOT_SPECS

func _detect_mode(args: PackedStringArray) -> String:
	if "--flythrough-boundary" in args or "--flythrough=boundary" in args:
		return MODE_BOUNDARY
	if "--flythrough-scene" in args or "--flythrough=scene" in args:
		return MODE_SCENE
	if "--flythrough" in args:
		return MODE_SCENE
	return ""

func _land_point(offset: Vector2) -> Vector3:
	if not _world:
		return Vector3.ZERO
	var bx := int(round(_spawn.x + offset.x))
	var bz := int(round(_spawn.z + offset.y))
	var land := _world.call("nearest_land_block", bx, bz) as Vector2
	var y := float(_world.call("sample_surface_height", int(land.x), int(land.y)))
	return Vector3(land.x + 0.5, y, land.y + 0.5)

func _apply_shot(i: int) -> void:
	var shot := _shots[i]
	_player.position              = shot["camera"]
	_player.velocity              = Vector3.ZERO
	_player.rotation_degrees.y    = shot["yaw"]
	_head.rotation_degrees.x      = shot["pitch"]

func _capture(i: int) -> void:
	var path := "%s/frame_%03d.png" % [_screenshot_dir, i + 1]
	print("Flythrough: capturing %s" % path)
	_save_screenshot.call_deferred(i, path)

func _save_screenshot(i: int, path: String) -> void:
	var img := get_viewport().get_texture().get_image()
	img.save_png(path)
	_current = i + 1
	_timer = 0.0
	if _current >= _shots.size():
		_phase = "exiting"
	else:
		_phase = "holding"
		_apply_shot(_current)
