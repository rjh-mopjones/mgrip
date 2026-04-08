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
var ocean_mask: PackedByteArray = PackedByteArray()
var _collision_body: StaticBody3D

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
	state = ChunkState.MESHING
	var mesh_start := Time.get_ticks_usec()
	var result := builder.build_terrain(biome_map, self, lod)
	var mesh_ms := (Time.get_ticks_usec() - mesh_start) / 1000.0
	heights = result["heights"]
	collision_heights = result["collision_heights"]
	ocean_mask = result["ocean_mask"]
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
	state = ChunkState.MESHING
	var mesh_start := Time.get_ticks_usec()
	var result := builder.build_terrain_from_mesh_data(mesh_data, self)
	var mesh_ms := (Time.get_ticks_usec() - mesh_start) / 1000.0
	heights = result["heights"]
	collision_heights = result["collision_heights"]
	ocean_mask = result["ocean_mask"]
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
