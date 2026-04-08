extends RefCounted
class_name ChunkStreamer

const DEFAULT_LOD := "LOD0"
const STREAM_RADIUS := 1
const RETAIN_RADIUS := 2
const WorldChunkScript = preload("res://scripts/world/world_chunk.gd")

var _terrain_root: Node3D
var _anchor_chunk: Vector2i = Vector2i.ZERO
var _current_center_chunk: Vector2i = Vector2i.ZERO
var _builder := VoxelMeshBuilder.new()
var _metrics
var _chunks: Dictionary = {}
var _pending_coords: Array[Vector2i] = []

func setup(terrain_root: Node3D, metrics, anchor_chunk: Vector2i) -> void:
	_terrain_root = terrain_root
	_metrics = metrics
	_anchor_chunk = anchor_chunk
	_current_center_chunk = anchor_chunk

func bootstrap_chunk(chunk_coord: Vector2i, collision_enabled: bool = true):
	_current_center_chunk = chunk_coord
	return _activate_chunk(chunk_coord, DEFAULT_LOD, collision_enabled)

func get_chunk(chunk_coord: Vector2i):
	return _chunks.get(_chunk_key(chunk_coord))

func has_chunk(chunk_coord: Vector2i) -> bool:
	return _chunks.has(_chunk_key(chunk_coord))

func active_counts_by_lod() -> Dictionary:
	var counts := {}
	for chunk in _chunks.values():
		var lod := String(chunk.lod)
		counts[lod] = int(counts.get(lod, 0)) + 1
	return counts

func pending_count() -> int:
	return _pending_coords.size()

func update_streaming(center_chunk: Vector2i) -> void:
	_current_center_chunk = center_chunk
	var center = get_chunk(center_chunk)
	if center == null:
		center = _activate_chunk(center_chunk, DEFAULT_LOD, true)
	_rebuild_pending(center_chunk)
	_update_collision_focus(center_chunk)
	_process_one_pending(center_chunk)
	_unload_distant_chunks(center_chunk)
	_metrics.update_runtime_state(active_counts_by_lod(), pending_count())

func _activate_chunk(chunk_coord: Vector2i, lod: String, collision_enabled: bool):
	var key := _chunk_key(chunk_coord)
	if _chunks.has(key):
		var existing = _chunks[key]
		existing.set_collision_enabled(collision_enabled)
		return existing

	var world_chunk = WorldChunkScript.new()
	world_chunk.configure(chunk_coord, _anchor_chunk, lod)
	world_chunk.state = WorldChunkScript.ChunkState.GENERATING

	var sample = _metrics.begin_activation(chunk_coord, lod)
	var generation_start := Time.get_ticks_usec()
	var biome_map := GenerationManager.generate_runtime_chunk(chunk_coord)
	_metrics.set_phase_ms(
		sample,
		"generation_ms",
		(Time.get_ticks_usec() - generation_start) / 1000.0
	)

	var build_result = world_chunk.activate_from_biome_map(biome_map, _builder, collision_enabled)
	_metrics.set_phase_ms(sample, "mesh_ms", float(build_result["mesh_ms"]))
	_metrics.set_phase_ms(sample, "collision_ms", float(build_result["collision_ms"]))

	var attach_start := Time.get_ticks_usec()
	_terrain_root.add_child(world_chunk)
	_metrics.set_phase_ms(sample, "attach_ms", (Time.get_ticks_usec() - attach_start) / 1000.0)

	_chunks[key] = world_chunk
	_metrics.finish_activation(sample)
	_metrics.update_runtime_state(active_counts_by_lod(), pending_count())
	return world_chunk

func _rebuild_pending(center_chunk: Vector2i) -> void:
	var pending: Array[Vector2i] = []
	for chunk_coord in _desired_chunk_order(center_chunk, STREAM_RADIUS):
		if chunk_coord == center_chunk:
			continue
		if has_chunk(chunk_coord):
			continue
		pending.append(chunk_coord)
	_pending_coords = pending

func _desired_chunk_order(center_chunk: Vector2i, radius: int) -> Array[Vector2i]:
	var coords: Array[Vector2i] = [center_chunk]
	for ring in range(1, radius + 1):
		for dz in range(-ring, ring + 1):
			for dx in range(-ring, ring + 1):
				if maxi(absi(dx), absi(dz)) != ring:
					continue
				coords.append(center_chunk + Vector2i(dx, dz))
	return coords

func _update_collision_focus(center_chunk: Vector2i) -> void:
	for chunk in _chunks.values():
		chunk.set_collision_enabled(chunk.chunk_coord == center_chunk)

func _process_one_pending(center_chunk: Vector2i) -> void:
	if _pending_coords.is_empty():
		return
	var next_chunk: Vector2i = _pending_coords[0]
	_pending_coords.remove_at(0)
	if has_chunk(next_chunk):
		return
	_activate_chunk(next_chunk, DEFAULT_LOD, next_chunk == center_chunk)

func _unload_distant_chunks(center_chunk: Vector2i) -> void:
	var unload_keys: Array[String] = []
	for key in _chunks.keys():
		var chunk = _chunks[key]
		var dx := absi(chunk.chunk_coord.x - center_chunk.x)
		var dy := absi(chunk.chunk_coord.y - center_chunk.y)
		if maxi(dx, dy) <= RETAIN_RADIUS:
			continue
		unload_keys.append(String(key))

	for key in unload_keys:
		var chunk = _chunks[key]
		chunk.begin_unload()
		_chunks.erase(key)
		chunk.queue_free()

func _chunk_key(chunk_coord: Vector2i) -> String:
	return "%d,%d" % [chunk_coord.x, chunk_coord.y]
