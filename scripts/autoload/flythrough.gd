extends Node

## Automated flythrough for visual testing.
## Run with:
##   godot --flythrough            # scenic landscape shots
##   godot --flythrough-boundary   # seam / chunk-boundary shots
##   godot --flythrough-crossing   # physical seam crossing test
##   godot --flythrough-flight     # high aerial horizon shots
## Screenshots saved to:
##   /tmp/mgrip_flythrough/scene/frame_001.png … frame_NNN.png
##   /tmp/mgrip_flythrough/boundary/frame_001.png … frame_NNN.png
##   /tmp/mgrip_flythrough/crossing/frame_001.png … frame_NNN.png
##   /tmp/mgrip_flythrough/flight/frame_001.png … frame_NNN.png
## Exits automatically when done.

const BASE_SCREENSHOT_DIR := "/tmp/mgrip_flythrough"
const MODE_SCENE := "scene"
const MODE_BOUNDARY := "boundary"
const MODE_CROSSING := "crossing"
const MODE_FLIGHT := "flight"
const WARMUP_SECS    := 2.5   # time for terrain to generate + player to spawn
const HOLD_SECS      := 1.2   # seconds at each waypoint before screenshot
const EXIT_DELAY     := 1.5   # flush time after last screenshot
const CLEARANCE      := 8.0
const FLIGHT_CAMERA_FAR := 6144.0
const CROSSING_SETTLE_SECS := 0.5
const CROSSING_SPEED := 8.0
const CROSSING_ARRIVE_DISTANCE := 0.9
const CROSSING_ROUTE_TIMEOUT := 5.0
const CROSSING_FALL_MARGIN := 2.0
const CROSSING_SEARCH_SENTINEL := 1000000.0

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

const FLIGHT_SHOT_SPECS: Array[Dictionary] = [
	{"focus": Vector2(   0,    0), "camera": Vector2(-420, -220), "height": 180.0},
	{"focus": Vector2( 320,  180), "camera": Vector2(-120,   40), "height": 140.0},
	{"focus": Vector2(-280,  260), "camera": Vector2(  80, -140), "height": 160.0},
	{"focus": Vector2( 180, -340), "camera": Vector2(-220, -120), "height": 170.0},
	{"focus": Vector2(-360, -220), "camera": Vector2( 120,  180), "height": 150.0},
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

const CROSSING_SEAM_SPECS: Array[Dictionary] = [
	{"name": "east", "axis": "x", "seam_offset": 256.0, "search_min": -180, "search_max": 180, "search_step": 12, "runout": 12.0, "height": 2.8, "pitch": -7.0},
	{"name": "south", "axis": "z", "seam_offset": 256.0, "search_min": -180, "search_max": 180, "search_step": 12, "runout": 12.0, "height": 2.8, "pitch": -9.0},
]

var _active  := false
var _player:  CharacterBody3D
var _head:    Node3D
var _camera:  Camera3D
var _world:   Node3D
var _spawn:   Vector3
var _shots:   Array[Dictionary] = []
var _routes:  Array[Dictionary] = []
var _mode    := ""
var _screenshot_dir := BASE_SCREENSHOT_DIR
var _phase    := "warmup"
var _timer    := 0.0
var _current  := 0
var _current_route := 0
var _frame_count := 0
var _crossing_failed := false

func _ready() -> void:
	_mode = _detect_mode(_all_cmdline_args())
	if _mode.is_empty():
		return
	_active = true
	_screenshot_dir = "%s/%s" % [BASE_SCREENSHOT_DIR, _mode]
	DirAccess.make_dir_recursive_absolute(_screenshot_dir)
	print("=== FLYTHROUGH MODE (%s) ===  output: %s" % [_mode, _screenshot_dir])

func _all_cmdline_args() -> PackedStringArray:
	var args := PackedStringArray()
	args.append_array(OS.get_cmdline_args())
	args.append_array(OS.get_cmdline_user_args())
	return args

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
		_camera = _head.get_node("Camera3D") as Camera3D
		_world  = get_tree().root.find_child("World", true, false) as Node3D
		_player.set_process_unhandled_input(false)
		if _player.has_method("clear_scripted_motion"):
			_player.call("clear_scripted_motion")
		if _mode == MODE_FLIGHT and _camera:
			_camera.far = FLIGHT_CAMERA_FAR
		if _mode != MODE_CROSSING:
			_player.set_physics_process(false)
		return

	_timer += delta
	if _mode == MODE_CROSSING:
		_process_crossing()
		return

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
				_finish_run()

func _process_crossing() -> void:
	match _phase:
		"warmup":
			if _timer >= WARMUP_SECS:
				_spawn = _player.position
				_routes = _build_crossing_routes()
				if _routes.is_empty():
					push_error("Flythrough: failed to build any crossing routes")
					get_tree().quit()
					return
				_current_route = 0
				_begin_crossing_route(_current_route)

		"crossing_settle":
			if _timer >= CROSSING_SETTLE_SECS:
				_capture_frame_now()
				var route: Dictionary = _routes[_current_route]
				_player.call("set_scripted_motion", route["direction"], float(route["speed"]))
				_phase = "crossing_run"
				_timer = 0.0

		"crossing_run":
			_monitor_crossing_route()

		"exiting":
			if _timer >= EXIT_DELAY:
				_finish_run()

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
	if _mode == MODE_FLIGHT:
		return FLIGHT_SHOT_SPECS
	if _mode == MODE_BOUNDARY:
		return BOUNDARY_SHOT_SPECS
	return SCENE_SHOT_SPECS

func _detect_mode(args: PackedStringArray) -> String:
	if "--flythrough-flight" in args or "--flythrough=flight" in args:
		return MODE_FLIGHT
	if "--flythrough-crossing" in args or "--flythrough=crossing" in args:
		return MODE_CROSSING
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
	var path := _next_capture_path()
	print("Flythrough: capturing %s" % path)
	_save_screenshot.call_deferred(i, path)

func _save_screenshot(i: int, path: String) -> void:
	var result := capture_screenshot_to_path(path)
	if not bool(result.get("ok", false)):
		push_error("Flythrough: failed to capture %s" % path)
	_frame_count += 1
	_current = i + 1
	_timer = 0.0
	if _current >= _shots.size():
		_phase = "exiting"
	else:
		_phase = "holding"
		_apply_shot(_current)

func _capture_frame_now() -> void:
	var path := _next_capture_path()
	print("Flythrough: capturing %s" % path)
	var result := capture_screenshot_to_path(path)
	if not bool(result.get("ok", false)):
		push_error("Flythrough: failed to capture %s" % path)
	_frame_count += 1

func capture_screenshot_to_path(path: String) -> Dictionary:
	var resolved_path := path
	if resolved_path.is_empty():
		return {
			"ok": false,
			"error_code": "empty_path",
			"error": "Path is empty.",
		}
	var absolute_path := ProjectSettings.globalize_path(resolved_path)
	DirAccess.make_dir_recursive_absolute(absolute_path.get_base_dir())
	if DisplayServer.get_name() == "headless":
		return {
			"ok": false,
			"path": resolved_path,
			"absolute_path": absolute_path,
			"error_code": "headless_screenshot_unavailable",
			"error": "Screenshot capture is unavailable when Godot is running with the headless display driver.",
		}
	var viewport := get_viewport()
	if viewport == null:
		return {
			"ok": false,
			"path": resolved_path,
			"absolute_path": absolute_path,
			"error_code": "viewport_unavailable",
			"error": "Viewport is unavailable.",
		}
	var texture := viewport.get_texture()
	if texture == null:
		return {
			"ok": false,
			"path": resolved_path,
			"absolute_path": absolute_path,
			"error_code": "viewport_texture_unavailable",
			"error": "Viewport texture is unavailable for screenshot capture.",
		}
	var image := texture.get_image()
	if image == null:
		return {
			"ok": false,
			"path": resolved_path,
			"absolute_path": absolute_path,
			"error_code": "viewport_image_unavailable",
			"error": "Viewport image is unavailable for screenshot capture.",
		}
	var error := image.save_png(resolved_path)
	return {
		"ok": error == OK,
		"path": resolved_path,
		"absolute_path": absolute_path,
		"error_code": "" if error == OK else "save_png_failed",
		"engine_error_code": error,
		"error": error_string(error) if error != OK else "",
	}

func is_active() -> bool:
	return _active

func current_mode() -> String:
	return _mode

func _build_crossing_routes() -> Array[Dictionary]:
	var routes: Array[Dictionary] = []
	for spec in CROSSING_SEAM_SPECS:
		var route := _best_crossing_route(spec)
		if route.is_empty():
			continue
		routes.append(route)
	return routes

func _best_crossing_route(spec: Dictionary) -> Dictionary:
	var best_score := CROSSING_SEARCH_SENTINEL
	var best_route := {}
	var axis := String(spec["axis"])
	var seam_offset := float(spec["seam_offset"])
	var runout := float(spec.get("runout", 12.0))
	var search_min := int(spec.get("search_min", -180))
	var search_max := int(spec.get("search_max", 180))
	var search_step := int(spec.get("search_step", 12))
	for lateral in range(search_min, search_max + 1, search_step):
		var start_offset := Vector2.ZERO
		var finish_offset := Vector2.ZERO
		if axis == "x":
			start_offset = Vector2(seam_offset - runout, float(lateral))
			finish_offset = Vector2(seam_offset + runout, float(lateral))
		else:
			start_offset = Vector2(float(lateral), seam_offset - runout)
			finish_offset = Vector2(float(lateral), seam_offset + runout)
		var score := _route_roughness_for_offsets(start_offset, finish_offset)
		if score >= best_score:
			continue
		var route := _build_crossing_route(
			String(spec["name"]),
			start_offset,
			finish_offset,
			float(spec.get("height", 2.8)),
			float(spec.get("pitch", -8.0)),
			float(spec.get("speed", CROSSING_SPEED)),
		)
		if route.is_empty():
			continue
		best_score = score
		route["roughness"] = score
		best_route = route
	return best_route

func _build_crossing_route(
		name: String,
		start_offset: Vector2,
		finish_offset: Vector2,
		height: float,
		pitch: float,
		speed: float) -> Dictionary:
	var start := _surface_point_at_offset(start_offset)
	var finish := _surface_point_at_offset(finish_offset)
	if start == Vector3.ZERO or finish == Vector3.ZERO:
		return {}
	start.y += height
	finish.y += height
	var direction := finish - start
	direction.y = 0.0
	if direction.length() < 1.0:
		return {}
	var forward := direction.normalized()
	return {
		"name": name,
		"start": start,
		"end": finish,
		"start_chunk": GenerationManager.scene_block_to_chunk_coord(
			GameState.anchor_chunk,
			start.x,
			start.z
		),
		"target_chunk": GenerationManager.scene_block_to_chunk_coord(
			GameState.anchor_chunk,
			finish.x,
			finish.z
		),
		"direction": forward,
		"speed": speed,
		"yaw": rad_to_deg(atan2(-forward.x, -forward.z)),
		"pitch": pitch,
		"midpoint_captured": false,
	}

func _route_roughness_for_offsets(start_offset: Vector2, finish_offset: Vector2) -> float:
	var min_y := INF
	var max_y := -INF
	var max_step := 0.0
	var previous_y := 0.0
	var has_previous := false
	for i in range(7):
		var t := float(i) / 6.0
		var point := _surface_point_at_offset(start_offset.lerp(finish_offset, t))
		if point == Vector3.ZERO:
			return CROSSING_SEARCH_SENTINEL
		min_y = minf(min_y, point.y)
		max_y = maxf(max_y, point.y)
		if has_previous:
			max_step = maxf(max_step, absf(point.y - previous_y))
		previous_y = point.y
		has_previous = true
	return (max_y - min_y) + max_step * 2.0

func _surface_point_at_offset(offset: Vector2) -> Vector3:
	if not _world:
		return Vector3.ZERO
	var block_x := int(round(_spawn.x + offset.x))
	var block_z := int(round(_spawn.z + offset.y))
	var y := float(_world.call("sample_surface_height", block_x, block_z))
	if y <= float(VoxelMeshBuilder.SEA_LEVEL_Y):
		return Vector3.ZERO
	return Vector3(block_x + 0.5, y, block_z + 0.5)

func _begin_crossing_route(route_index: int) -> void:
	var route: Dictionary = _routes[route_index]
	print("Flythrough: crossing route %d/%d (%s, roughness %.2f)" % [route_index + 1, _routes.size(), route["name"], float(route.get("roughness", 0.0))])
	_player.call("clear_scripted_motion")
	_player.position = route["start"]
	_player.velocity = Vector3.ZERO
	_player.rotation_degrees.y = float(route["yaw"])
	_head.rotation_degrees.x = float(route["pitch"])
	_phase = "crossing_settle"
	_timer = 0.0

func _monitor_crossing_route() -> void:
	var route: Dictionary = _routes[_current_route]
	if _player.position.y < _surface_height_beneath_player() - CROSSING_FALL_MARGIN:
		_crossing_failed = true
		_player.call("clear_scripted_motion")
		push_error("Flythrough crossing failed on route %s" % route["name"])
		_capture_frame_now()
		_phase = "exiting"
		_timer = 0.0
		return
	if _timer >= CROSSING_ROUTE_TIMEOUT:
		_crossing_failed = true
		_player.call("clear_scripted_motion")
		push_error("Flythrough crossing timed out on route %s" % route["name"])
		_capture_frame_now()
		_phase = "exiting"
		_timer = 0.0
		return

	if not bool(route["midpoint_captured"]) and _route_progress(route) >= 0.5:
		_capture_frame_now()
		route["midpoint_captured"] = true
		_routes[_current_route] = route

	if _route_crossed_target_chunk(route):
		_player.call("clear_scripted_motion")
		_capture_frame_now()
		_current_route += 1
		if _current_route >= _routes.size():
			_phase = "exiting"
			_timer = 0.0
			return
		_begin_crossing_route(_current_route)

func _route_progress(route: Dictionary) -> float:
	var start: Vector3 = route["start"]
	var finish: Vector3 = route["end"]
	var total := Vector2(finish.x - start.x, finish.z - start.z)
	var traveled := Vector2(_player.position.x - start.x, _player.position.z - start.z)
	var length_sq := maxf(total.length_squared(), 0.001)
	return clampf(traveled.dot(total) / length_sq, 0.0, 1.0)

func _route_crossed_target_chunk(route: Dictionary) -> bool:
	var current_chunk := GenerationManager.scene_block_to_chunk_coord(
		GameState.anchor_chunk,
		_player.position.x,
		_player.position.z,
	)
	return current_chunk == route["target_chunk"] and _route_progress(route) >= 0.6

func _surface_height_beneath_player() -> float:
	var block_x := int(round(_player.position.x - 0.5))
	var block_z := int(round(_player.position.z - 0.5))
	return float(_world.call("sample_surface_height", block_x, block_z))

func _next_capture_path() -> String:
	return "%s/frame_%03d.png" % [_screenshot_dir, _frame_count + 1]

func _finish_run() -> void:
	if _player and _player.has_method("clear_scripted_motion"):
		_player.call("clear_scripted_motion")
	var status := ""
	if _mode == MODE_CROSSING:
		status = " (%s)" % ["FAIL" if _crossing_failed else "PASS"]
	print("=== FLYTHROUGH DONE%s: %d frames saved to %s ===" % [status, _frame_count, _screenshot_dir])
	get_tree().quit()
