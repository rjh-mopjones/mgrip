extends Node3D
class_name WorldChunk

const COLLISION_HORIZONTAL_SCALE := float(VoxelMeshBuilder.CHUNK_SIZE) / float(VoxelMeshBuilder.CHUNK_SIZE - 1)

enum ChunkState {
	REQUESTED,
	GENERATING,
	MESHING,
	ACTIVE,
	UNLOADING,
}

var chunk_coord: Vector2i = Vector2i.ZERO
var lod: String = "LOD0"
var state: int = ChunkState.REQUESTED
var biome_map: MgBiomeMap
var heights: PackedInt32Array = PackedInt32Array()
var collision_heights: PackedFloat32Array = PackedFloat32Array()
var fluid_surface_mask: PackedByteArray = PackedByteArray()
var runtime_presentation: Dictionary = {}
var runtime_presentation_grids: Dictionary = {}
var planet_zone: int = -1
var atmosphere_class: int = -1
var water_state: int = -1
var landform_class: int = -1
var surface_palette_class: int = -1
var interestingness_score: float = 0.0
var _collision_body: StaticBody3D
var _render_resources_released := false

func configure(new_chunk_coord: Vector2i, anchor_chunk: Vector2i, lod_name: String = "LOD0") -> void:
	chunk_coord = new_chunk_coord
	lod = lod_name
	name = "Chunk_%d_%d_%s" % [chunk_coord.x, chunk_coord.y, lod]
	position = GenerationManager.chunk_coord_to_scene_origin(chunk_coord, anchor_chunk)

func activate_from_biome_map(
		new_biome_map: MgBiomeMap,
		builder: VoxelMeshBuilder,
		collision_enabled: bool) -> Dictionary:
	biome_map = new_biome_map
	_apply_runtime_presentation_bundle(new_biome_map.build_runtime_chunk_presentation_data())
	state = ChunkState.MESHING
	var mesh_start := Time.get_ticks_usec()
	var result := builder.build_terrain(
		biome_map,
		self,
		lod,
		runtime_presentation,
		runtime_presentation_grids,
	)
	var mesh_ms := (Time.get_ticks_usec() - mesh_start) / 1000.0
	heights = result["heights"]
	collision_heights = result["collision_heights"]
	fluid_surface_mask = result["fluid_surface_mask"]
	var collision_ms := 0.0
	if collision_enabled:
		var collision_start := Time.get_ticks_usec()
		set_collision_enabled(true)
		collision_ms = (Time.get_ticks_usec() - collision_start) / 1000.0
	state = ChunkState.ACTIVE
	return {
		"mesh_ms": mesh_ms,
		"collision_ms": collision_ms,
	}

func activate_from_chunk_data(
		new_biome_map: MgBiomeMap,
		mesh_data: Dictionary,
		builder: VoxelMeshBuilder,
		collision_enabled: bool) -> Dictionary:
	biome_map = new_biome_map
	_apply_runtime_presentation_bundle(new_biome_map.build_runtime_chunk_presentation_data())
	state = ChunkState.MESHING
	var mesh_start := Time.get_ticks_usec()
	var result := builder.build_terrain_from_mesh_data(
		mesh_data,
		self,
		lod,
		runtime_presentation,
		runtime_presentation_grids,
	)
	var mesh_ms := (Time.get_ticks_usec() - mesh_start) / 1000.0
	heights = result["heights"]
	collision_heights = result["collision_heights"]
	fluid_surface_mask = result["fluid_surface_mask"]
	var collision_ms := 0.0
	if collision_enabled:
		var collision_start := Time.get_ticks_usec()
		set_collision_enabled(true)
		collision_ms = (Time.get_ticks_usec() - collision_start) / 1000.0
	state = ChunkState.ACTIVE
	return {
		"mesh_ms": mesh_ms,
		"collision_ms": collision_ms,
	}

func set_collision_enabled(enabled: bool) -> void:
	if enabled:
		if _collision_body or collision_heights.is_empty():
			return
		if collision_heights.size() != VoxelMeshBuilder.CHUNK_SIZE * VoxelMeshBuilder.CHUNK_SIZE:
			return
		_collision_body = _build_collision_body()
		add_child(_collision_body)
		return
	if _collision_body:
		_collision_body.queue_free()
		_collision_body = null

func has_collision() -> bool:
	return _collision_body != null

func begin_unload() -> void:
	state = ChunkState.UNLOADING
	release_render_resources()

func release_render_resources() -> void:
	if _render_resources_released:
		return
	_render_resources_released = true
	set_collision_enabled(false)
	for child in get_children():
		if child is MeshInstance3D:
			var mesh_instance := child as MeshInstance3D
			mesh_instance.mesh = null
			mesh_instance.material_override = null
	biome_map = null
	heights = PackedInt32Array()
	collision_heights = PackedFloat32Array()
	fluid_surface_mask = PackedByteArray()
	runtime_presentation = {}
	runtime_presentation_grids = {}
	planet_zone = -1
	atmosphere_class = -1
	water_state = -1
	landform_class = -1
	surface_palette_class = -1
	interestingness_score = 0.0

func _apply_runtime_presentation_bundle(presentation_data: Dictionary) -> void:
	var summary: Dictionary = presentation_data.get("summary", {})
	runtime_presentation_grids = presentation_data.get("reduced_grids", {}).duplicate(true)
	runtime_presentation = summary.duplicate(true)
	if not runtime_presentation_grids.is_empty():
		runtime_presentation["reduced_grids"] = _reduced_grid_metadata(runtime_presentation_grids)
	var zone: Dictionary = runtime_presentation.get("planet_zone", {})
	var atmosphere: Dictionary = runtime_presentation.get("atmosphere_class", {})
	var water: Dictionary = runtime_presentation.get("water_state", {})
	var landform: Dictionary = runtime_presentation.get("landform_class", {})
	var surface_palette: Dictionary = runtime_presentation.get("surface_palette_class", {})
	planet_zone = int(zone.get("id", -1))
	atmosphere_class = int(atmosphere.get("id", -1))
	water_state = int(water.get("id", -1))
	landform_class = int(landform.get("id", -1))
	surface_palette_class = int(surface_palette.get("id", -1))
	interestingness_score = float(runtime_presentation.get("interestingness_score", 0.0))

func _reduced_grid_metadata(grids: Dictionary) -> Dictionary:
	return {
		"water_state_grid": _grid_metadata(grids.get("water_state_grid", {})),
		"landform_grid": _grid_metadata(grids.get("landform_grid", {})),
		"surface_palette_grid": _grid_metadata(grids.get("surface_palette_grid", {})),
	}

func _grid_metadata(grid: Dictionary) -> Dictionary:
	return {
		"width": int(grid.get("width", 0)),
		"height": int(grid.get("height", 0)),
		"digest": String(grid.get("digest", "")),
	}

func _build_collision_body() -> StaticBody3D:
	var shape_data := PackedFloat32Array()
	shape_data.resize(VoxelMeshBuilder.CHUNK_SIZE * VoxelMeshBuilder.CHUNK_SIZE)
	for i in shape_data.size():
		shape_data[i] = collision_heights[i] / COLLISION_HORIZONTAL_SCALE

	var hms := HeightMapShape3D.new()
	hms.map_width = VoxelMeshBuilder.CHUNK_SIZE
	hms.map_depth = VoxelMeshBuilder.CHUNK_SIZE
	hms.map_data = shape_data

	var body := StaticBody3D.new()
	body.name = "CollisionBody"
	var collision_shape := CollisionShape3D.new()
	collision_shape.shape = hms
	collision_shape.scale = Vector3.ONE * COLLISION_HORIZONTAL_SCALE
	collision_shape.position = Vector3(
		VoxelMeshBuilder.CHUNK_SIZE * 0.5,
		0.0,
		VoxelMeshBuilder.CHUNK_SIZE * 0.5,
	)
	body.add_child(collision_shape)
	return body
