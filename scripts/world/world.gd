extends Node3D

const ChunkMetricsScript = preload("res://scripts/world/chunk_metrics.gd")
const ChunkStreamerScript = preload("res://scripts/world/chunk_streamer.gd")
const WorldEnvironmentControllerScript = preload("res://scripts/world/world_environment_controller.gd")
const DEFAULT_WORLD_X := 440.0
const DEFAULT_WORLD_Y := 220.0

@export var world_x: float = DEFAULT_WORLD_X
@export var world_y: float = DEFAULT_WORLD_Y

@onready var _terrain_root: Node3D    = $TerrainRoot
@onready var _sun: DirectionalLight3D = $Sun
@onready var _world_environment: WorldEnvironment = $WorldEnvironment
@onready var _player: CharacterBody3D = $Player
@onready var _head: Node3D = $Player/Head
@onready var _camera: Camera3D = $Player/Head/Camera3D

var _map_overlay: MapOverlay
var _world_environment_controller = null
var _chunk_metrics = null
var _chunk_streamer = null
var _anchor_chunk: Vector2i = Vector2i.ZERO
var _last_map_chunk: Vector2i = Vector2i(1 << 20, 1 << 20)
var _last_prewarm_target: Vector2i = Vector2i(1 << 20, 1 << 20)
var _last_environment_chunk: Vector2i = Vector2i(1 << 20, 1 << 20)

func _ready() -> void:
	var window := get_window()
	if window:
		window.size_changed.connect(_on_window_size_changed)
	var launch_world_origin := _consume_launch_world_origin()
	_anchor_chunk = GenerationManager.world_origin_to_chunk_coord(
		launch_world_origin.x,
		launch_world_origin.y,
	)
	GameState.anchor_chunk = _anchor_chunk
	GameState.current_chunk = _anchor_chunk
	_chunk_metrics = ChunkMetricsScript.new()
	_chunk_streamer = ChunkStreamerScript.new()
	print("Generating runtime chunk [%d, %d]…" % [_anchor_chunk.x, _anchor_chunk.y])
	var t0 := Time.get_ticks_msec()

	_chunk_streamer.setup(_terrain_root, _chunk_metrics, _anchor_chunk)
	var boot_chunk = _chunk_streamer.bootstrap_chunk(_anchor_chunk, true)
	_chunk_streamer.update_streaming(_anchor_chunk)
	_log_height_stats(boot_chunk.heights)
	_place_player(boot_chunk)
	_setup_environment_controller(boot_chunk)
	_last_prewarm_target = _chunk_streamer.prewarm_target_chunk()
	if not _is_flythrough_run():
		_setup_map(boot_chunk)
	_register_agent_runtime()

	_chunk_metrics.maybe_print_summary()
	print("Chunk runtime ready in %.1fs" % ((Time.get_ticks_msec() - t0) / 1000.0))

func _exit_tree() -> void:
	var agent_runtime = get_node_or_null("/root/AgentRuntime")
	if agent_runtime != null and agent_runtime.has_method("unregister_world_runtime"):
		agent_runtime.call("unregister_world_runtime", self)

func _on_window_size_changed() -> void:
	var window := get_window()
	if window:
		print("world window_size=", window.size)

func _unhandled_input(event: InputEvent) -> void:
	if event.is_action_pressed("map") and _map_overlay:
		_map_overlay.toggle()
		Input.set_mouse_mode(
			Input.MOUSE_MODE_VISIBLE if _map_overlay.visible
			else Input.MOUSE_MODE_CAPTURED
		)
	elif _map_overlay and _map_overlay.visible and event is InputEventKey:
		var key_event := event as InputEventKey
		if key_event.pressed and not key_event.echo:
			if key_event.keycode == KEY_TAB or event.is_action_pressed("map_toggle"):
				_map_overlay.toggle_mode()

func _process(_delta: float) -> void:
	var current_chunk := GenerationManager.scene_block_to_chunk_coord(
		_anchor_chunk,
		_player.position.x,
		_player.position.z,
	)
	var player_forward := -_player.global_transform.basis.z
	if current_chunk != GameState.current_chunk:
		var had_prewarmed_ring: bool = _chunk_streamer.is_ring_ready(current_chunk)
		GameState.current_chunk = current_chunk
		print(
			"Player entered chunk [%d, %d]  prewarmed_ring=%s  pending=%d"
			% [
				current_chunk.x,
				current_chunk.y,
				"yes" if had_prewarmed_ring else "no",
				_chunk_streamer.pending_count(),
			]
		)
	_chunk_streamer.update_streaming(current_chunk, _player.position, _player.velocity, player_forward)
	var loaded_chunk = _chunk_streamer.get_chunk(current_chunk)
	_maybe_update_environment(current_chunk, loaded_chunk)
	var prewarm_target: Vector2i = _chunk_streamer.prewarm_target_chunk()
	if prewarm_target != _last_prewarm_target and prewarm_target != current_chunk:
		print(
			"Prewarming next center [%d, %d]  ring_ready=%s  pending=%d"
			% [
				prewarm_target.x,
				prewarm_target.y,
				"yes" if _chunk_streamer.is_ring_ready(prewarm_target) else "no",
				_chunk_streamer.pending_count(),
			]
		)
	_last_prewarm_target = prewarm_target
	if _map_overlay:
		if loaded_chunk and current_chunk != _last_map_chunk:
			_map_overlay.update_local_chunk(loaded_chunk.biome_map, current_chunk)
			_last_map_chunk = current_chunk
		_map_overlay.refresh(
			_player.position,
			current_chunk,
			_chunk_streamer.active_counts_by_lod(),
			_player.rotation.y,
			{
				"pending": _chunk_streamer.pending_count(),
				"prewarm_target": _chunk_streamer.prewarm_target_chunk(),
				"horizon": _chunk_streamer.horizon_runtime_state(),
				"window": _chunk_streamer.loaded_chunk_window(current_chunk),
			}
		)
	_chunk_metrics.update_runtime_state(
		_chunk_streamer.active_counts_by_lod(),
		_chunk_streamer.pending_count(),
	)
	_chunk_metrics.set_horizon_state(_chunk_streamer.horizon_runtime_state())
	_chunk_metrics.maybe_print_summary()

# ── Helpers ───────────────────────────────────────────────────────────────────

func _consume_launch_world_origin() -> Vector2:
	var fallback_origin := Vector2(world_x, world_y)
	if not GameState.has_pending_launch:
		GameState.record_runtime_launch(
			GameState.LaunchMode.DIRECT_COORD,
			fallback_origin,
			GenerationManager.world_origin_to_chunk_coord(fallback_origin.x, fallback_origin.y)
		)
		return fallback_origin

	var launch_origin := fallback_origin
	var launch_mode := GameState.launch_mode
	var launch_chunk := GenerationManager.world_origin_to_chunk_coord(fallback_origin.x, fallback_origin.y)
	match GameState.launch_mode:
		GameState.LaunchMode.SELECTED_CHUNK:
			launch_origin = GenerationManager.chunk_coord_to_world_origin(GameState.launch_chunk)
			launch_chunk = GameState.launch_chunk
		_:
			launch_origin = GameState.launch_world_origin
			launch_chunk = GenerationManager.world_origin_to_chunk_coord(launch_origin.x, launch_origin.y)
	GameState.record_runtime_launch(launch_mode, launch_origin, launch_chunk)
	GameState.clear_launch_request()
	return launch_origin

func _setup_map(chunk) -> void:
	_map_overlay = MapOverlay.new()
	add_child(_map_overlay)
	_map_overlay.setup(chunk.biome_map, _anchor_chunk, chunk.chunk_coord)
	_map_overlay.attach_hud.call_deferred(self)

func _setup_environment_controller(chunk) -> void:
	_world_environment_controller = WorldEnvironmentControllerScript.new()
	_world_environment_controller.setup(_world_environment, _sun)
	_maybe_update_environment(chunk.chunk_coord, chunk)

func _maybe_update_environment(chunk_coord: Vector2i, chunk) -> void:
	if _world_environment_controller == null or chunk == null:
		return
	if chunk_coord == _last_environment_chunk and not chunk.runtime_presentation.is_empty():
		return
	_world_environment_controller.apply_runtime_presentation(chunk.runtime_presentation)
	_last_environment_chunk = chunk_coord

func _place_player(chunk) -> void:
	var cx: int = VoxelMeshBuilder.CHUNK_SIZE / 2
	var cz: int = VoxelMeshBuilder.CHUNK_SIZE / 2
	var land := _find_land_block(cx, cz, chunk.fluid_surface_mask)
	var surface_y: int
	var chunk_origin := GenerationManager.chunk_coord_to_scene_origin(chunk.chunk_coord, _anchor_chunk)
	if land.x >= 0:
		surface_y = chunk.heights[int(land.y) * VoxelMeshBuilder.CHUNK_SIZE + int(land.x)] + 1
		_player.position = Vector3(
			chunk_origin.x + land.x + 0.5,
			surface_y + 3.0,
			chunk_origin.z + land.y + 0.5,
		)
	else:
		# Entire chunk is covered by non-standable fluid.
		push_warning("Entire chunk is covered by fluid surface — spawning above center")
		_player.position = Vector3(
			chunk_origin.x + cx + 0.5,
			VoxelMeshBuilder.SEA_LEVEL_Y + 8.0,
			chunk_origin.z + cz + 0.5,
		)

func sample_surface_height(block_x: int, block_z: int) -> float:
	var chunk = _loaded_chunk_for_scene_block(block_x, block_z)
	if chunk == null or chunk.heights.is_empty():
		return 0.0
	var block := GenerationManager.scene_block_to_local_block(block_x, block_z)
	return float(chunk.heights[block.y * VoxelMeshBuilder.CHUNK_SIZE + block.x]) + 1.0

func nearest_land_block(block_x: int, block_z: int) -> Vector2:
	var chunk_coord := GenerationManager.scene_block_to_chunk_coord(_anchor_chunk, block_x, block_z)
	var chunk = _chunk_streamer.get_chunk(chunk_coord)
	if chunk == null or chunk.fluid_surface_mask.is_empty():
		return Vector2(block_x, block_z)
	var block := GenerationManager.scene_block_to_local_block(block_x, block_z)
	var local_land := _find_land_block(block.x, block.y, chunk.fluid_surface_mask)
	if local_land.x < 0:
		return Vector2(block_x, block_z)
	var chunk_origin := GenerationManager.chunk_coord_to_scene_origin(chunk_coord, _anchor_chunk)
	return Vector2(
		chunk_origin.x + local_land.x,
		chunk_origin.z + local_land.y,
	)

## Print micro-scale heightmap statistics to diagnose detail_level=2 noise.
## Uses the already-computed heights array (zero extra FFI calls).
func _log_height_stats(heights: PackedInt32Array) -> void:
	var total      := heights.size()
	var scale      := VoxelMeshBuilder.HEIGHT_SCALE
	var sea_blocks := VoxelMeshBuilder.SEA_LEVEL_Y
	var min_b      := heights[0]
	var max_b      := heights[0]
	var sum        := 0
	var land_count := 0
	var buckets    := PackedInt32Array([0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
	for b in heights:
		if b < min_b: min_b = b
		if b > max_b: max_b = b
		sum += b
		if b > sea_blocks:
			land_count += 1
		# Map block height back to [-1, 1] for bucket
		var hv      := b / scale
		var bucket  := clampi(int((hv + 1.0) * 5.0), 0, 9)
		buckets[bucket] += 1
	print("── Micro heightmap stats (detail_level=2) ──")
	print("  blocks : [%d, %d]   mean: %.1f" % [min_b, max_b, float(sum) / total])
	print("  land   : %.1f%%" % [100.0 * land_count / total])
	for i in 10:
		var pct := 100.0 * buckets[i] / total
		var lo  := -1.0 + i * 0.2
		print("    [%+.1f, %+.1f): %5.1f%%" % [lo, lo + 0.2, pct])
	print("────────────────────────────────────────────")

## Spiral search outward from (cx, cz) until a standable block is found.
## Returns Vector2(block_x, block_z) or Vector2(-1, -1) if none found.
func _find_land_block(cx: int, cz: int, fluid_surface_mask: PackedByteArray) -> Vector2:
	var size := VoxelMeshBuilder.CHUNK_SIZE
	if not fluid_surface_mask[cz * size + cx]:
		return Vector2(cx, cz)
	var step := 1
	while step < size:
		for dx in range(-step, step + 1):
			for dz_off in [-step, step]:
				var x: int = cx + dx
				var z: int = cz + dz_off
				if x >= 0 and x < size and z >= 0 and z < size:
					if not fluid_surface_mask[z * size + x]:
						return Vector2(x, z)
		for dz in range(-step + 1, step):
			for dx_off in [-step, step]:
				var x: int = cx + dx_off
				var z: int = cz + dz
				if x >= 0 and x < size and z >= 0 and z < size:
					if not fluid_surface_mask[z * size + x]:
						return Vector2(x, z)
		step += 4
	return Vector2(-1, -1)

func _loaded_chunk_for_scene_block(block_x: int, block_z: int):
	var chunk_coord := GenerationManager.scene_block_to_chunk_coord(_anchor_chunk, block_x, block_z)
	return _chunk_streamer.get_chunk(chunk_coord)

func is_chunk_loaded(chunk_coord: Vector2i) -> bool:
	return _chunk_streamer != null and _chunk_streamer.has_chunk(chunk_coord)

func get_chunk_state(chunk_coord: Vector2i) -> Dictionary:
	if _chunk_streamer == null:
		return {
			"chunk_coord": chunk_coord,
			"loaded": false,
		}
	return _chunk_streamer.chunk_state(chunk_coord)

func get_current_chunk_state() -> Dictionary:
	return get_chunk_state(GameState.current_chunk)

func get_chunk_runtime_presentation(chunk_coord: Vector2i) -> Dictionary:
	var chunk = _chunk_streamer.get_chunk(chunk_coord) if _chunk_streamer != null else null
	return chunk.runtime_presentation if chunk != null else {}

func get_current_runtime_presentation() -> Dictionary:
	return get_chunk_runtime_presentation(GameState.current_chunk)

## Macro-truth sampler bridge for the agent runtime. Reads from the cached
## `MACRO_SEMANTICS` singleton in Rust via `GenerationManager.sample_macro_point`
## so agent observations can validate runtime chunks against the macromap
## without re-generating macro data per call.
func sample_macro_semantics(world_x: float, world_y: float) -> Dictionary:
	return GenerationManager.sample_macro_point(world_x, world_y)

func get_player_node() -> CharacterBody3D:
	return _player

func get_head_node() -> Node3D:
	return _head

func get_camera_node() -> Camera3D:
	return _camera

func get_chunk_streamer():
	return _chunk_streamer

func _register_agent_runtime() -> void:
	var agent_runtime = get_node_or_null("/root/AgentRuntime")
	if agent_runtime == null:
		return
	if agent_runtime.has_method("register_world_runtime"):
		agent_runtime.call("register_world_runtime", self, _player, _head, _camera, _chunk_streamer)

func _is_flythrough_run() -> bool:
	var args := PackedStringArray()
	args.append_array(OS.get_cmdline_args())
	args.append_array(OS.get_cmdline_user_args())
	for arg in args:
		if String(arg).begins_with("--flythrough"):
			return true
	return false
