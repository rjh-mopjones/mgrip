extends Node3D

@export var world_x: float = 440.0
@export var world_y: float = 220.0

@onready var _terrain_root: Node3D    = $TerrainRoot
@onready var _player: CharacterBody3D = $Player

var _map_overlay: MapOverlay
var _heights: PackedInt32Array
var _ocean: PackedByteArray

func _ready() -> void:
	print("Generating chunk at world (%.1f, %.1f)…" % [world_x, world_y])
	var t0 := Time.get_ticks_msec()

	var biome_map := GenerationManager.generate_chunk(world_x, world_y)

	var builder := VoxelMeshBuilder.new()
	var result  := builder.build_terrain(biome_map, _terrain_root)
	_heights = result["heights"]
	_ocean   = result["ocean_mask"]

	_log_height_stats(_heights)

	_add_terrain_collision(_heights)
	_place_player(_heights, _ocean)
	if "--flythrough" not in OS.get_cmdline_args():
		_setup_map(biome_map)

	print("Chunk ready in %.1fs" % ((Time.get_ticks_msec() - t0) / 1000.0))

func _unhandled_input(event: InputEvent) -> void:
	if event.is_action_pressed("map"):
		_map_overlay.toggle()
		Input.set_mouse_mode(
			Input.MOUSE_MODE_VISIBLE if _map_overlay.visible
			else Input.MOUSE_MODE_CAPTURED
		)

func _process(_delta: float) -> void:
	if _map_overlay:
		_map_overlay.refresh(_player.position)

# ── Helpers ───────────────────────────────────────────────────────────────────

func _setup_map(biome_map: MgBiomeMap) -> void:
	_map_overlay = MapOverlay.new()
	add_child(_map_overlay)
	_map_overlay.setup(biome_map, world_x, world_y)
	_map_overlay.attach_hud.call_deferred(self)

func _add_terrain_collision(heights: PackedInt32Array) -> void:
	var shape_data := PackedFloat32Array()
	shape_data.resize(VoxelMeshBuilder.CHUNK_SIZE * VoxelMeshBuilder.CHUNK_SIZE)
	for i in shape_data.size():
		shape_data[i] = float(heights[i]) + 1.0

	var hms := HeightMapShape3D.new()
	hms.map_width = VoxelMeshBuilder.CHUNK_SIZE
	hms.map_depth = VoxelMeshBuilder.CHUNK_SIZE
	hms.map_data  = shape_data

	var body := StaticBody3D.new()
	var cs   := CollisionShape3D.new()
	cs.shape    = hms
	cs.position = Vector3(
		VoxelMeshBuilder.CHUNK_SIZE * 0.5,
		0.0,
		VoxelMeshBuilder.CHUNK_SIZE * 0.5,
	)
	body.add_child(cs)
	add_child(body)

func _place_player(heights: PackedInt32Array, ocean: PackedByteArray) -> void:
	var cx: int = VoxelMeshBuilder.CHUNK_SIZE / 2
	var cz: int = VoxelMeshBuilder.CHUNK_SIZE / 2
	var land := _find_land_block(cx, cz, ocean)
	var surface_y: int
	if land.x >= 0:
		surface_y = heights[int(land.y) * VoxelMeshBuilder.CHUNK_SIZE + int(land.x)] + 1
		_player.position = Vector3(land.x + 0.5, surface_y + 3.0, land.y + 0.5)
	else:
		# Entire chunk is ocean — float above sea level
		push_warning("Entire chunk is ocean — spawning above water")
		_player.position = Vector3(cx + 0.5, VoxelMeshBuilder.SEA_LEVEL_Y + 8.0, cz + 0.5)

func sample_surface_height(block_x: int, block_z: int) -> float:
	if _heights.is_empty():
		return 0.0
	var bx := clampi(block_x, 0, VoxelMeshBuilder.CHUNK_SIZE - 1)
	var bz := clampi(block_z, 0, VoxelMeshBuilder.CHUNK_SIZE - 1)
	return float(_heights[bz * VoxelMeshBuilder.CHUNK_SIZE + bx]) + 1.0

func nearest_land_block(block_x: int, block_z: int) -> Vector2:
	if _ocean.is_empty():
		return Vector2(block_x, block_z)
	var bx := clampi(block_x, 0, VoxelMeshBuilder.CHUNK_SIZE - 1)
	var bz := clampi(block_z, 0, VoxelMeshBuilder.CHUNK_SIZE - 1)
	return _find_land_block(bx, bz, _ocean)

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

## Spiral search outward from (cx, cz) until a non-ocean block is found.
## Returns Vector2(block_x, block_z) or Vector2(-1, -1) if none found.
func _find_land_block(cx: int, cz: int, ocean: PackedByteArray) -> Vector2:
	var size := VoxelMeshBuilder.CHUNK_SIZE
	if not ocean[cz * size + cx]:
		return Vector2(cx, cz)
	var step := 1
	while step < size:
		for dx in range(-step, step + 1):
			for dz_off in [-step, step]:
				var x: int = cx + dx
				var z: int = cz + dz_off
				if x >= 0 and x < size and z >= 0 and z < size:
					if not ocean[z * size + x]:
						return Vector2(x, z)
		for dz in range(-step + 1, step):
			for dx_off in [-step, step]:
				var x: int = cx + dx_off
				var z: int = cz + dz
				if x >= 0 and x < size and z >= 0 and z < size:
					if not ocean[z * size + x]:
						return Vector2(x, z)
		step += 4
	return Vector2(-1, -1)
