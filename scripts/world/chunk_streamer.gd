extends RefCounted
class_name ChunkStreamer

const ACTIVE_LOD := GenerationManager.LOD0_NAME
const STREAM_RADIUS := 1
const COLLISION_RADIUS := 1
const RETAIN_RADIUS := 2
const PREWARM_EDGE_MARGIN := 72.0
const PREWARM_MIN_SPEED := 2.0
const MAX_CONCURRENT_JOBS := 8
const MAX_READY_ATTACHES_PER_FRAME := 1
const WorldChunkScript = preload("res://scripts/world/world_chunk.gd")

var _terrain_root: Node3D
var _anchor_chunk: Vector2i = Vector2i.ZERO
var _current_center_chunk: Vector2i = Vector2i.ZERO
var _builder := VoxelMeshBuilder.new()
var _metrics
var _chunks: Dictionary = {}
var _pending_requests: Array[Dictionary] = []
var _inflight_jobs: Dictionary = {}
var _prewarm_target_chunk: Vector2i = Vector2i.ZERO

func setup(terrain_root: Node3D, metrics, anchor_chunk: Vector2i) -> void:
	_terrain_root = terrain_root
	_metrics = metrics
	_anchor_chunk = anchor_chunk
	_current_center_chunk = anchor_chunk

func bootstrap_chunk(chunk_coord: Vector2i, collision_enabled: bool = true):
	_current_center_chunk = chunk_coord
	return _activate_chunk_sync(chunk_coord, ACTIVE_LOD, collision_enabled)

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
	return _pending_requests.size() + _inflight_jobs.size()

func update_streaming(center_chunk: Vector2i, player_position: Vector3 = Vector3.ZERO, player_velocity: Vector3 = Vector3.ZERO) -> void:
	_current_center_chunk = center_chunk
	_prewarm_target_chunk = _predict_next_center_chunk(center_chunk, player_position, player_velocity)
	_attach_ready_jobs(center_chunk)
	_ensure_chunk_sync(center_chunk, ACTIVE_LOD, true)
	_rebuild_pending(center_chunk, _prewarm_target_chunk)
	_start_pending_jobs()
	_update_collision_focus(center_chunk)
	_unload_distant_chunks(center_chunk)
	_metrics.update_runtime_state(active_counts_by_lod(), pending_count())

func prewarm_target_chunk() -> Vector2i:
	return _prewarm_target_chunk

func is_ring_ready(center_chunk: Vector2i, radius: int = STREAM_RADIUS) -> bool:
	for chunk_coord in _desired_chunk_order(center_chunk, radius):
		if not _has_matching_chunk(chunk_coord, ACTIVE_LOD):
			return false
	return true

func _activate_chunk_sync(chunk_coord: Vector2i, lod: String, collision_enabled: bool):
	var key := _chunk_key(chunk_coord)
	if _chunks.has(key):
		var existing = _chunks[key]
		if String(existing.lod) != lod:
			existing.begin_unload()
			_chunks.erase(key)
			existing.queue_free()
		else:
			existing.set_collision_enabled(collision_enabled)
			return existing

	var world_chunk = WorldChunkScript.new()
	world_chunk.configure(chunk_coord, _anchor_chunk, lod)
	world_chunk.state = WorldChunkScript.ChunkState.GENERATING

	var sample = _metrics.begin_activation(chunk_coord, lod)
	var generation_start := Time.get_ticks_usec()
	var biome_map := GenerationManager.generate_runtime_chunk_for_lod(chunk_coord, lod)
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

func _ensure_chunk_sync(chunk_coord: Vector2i, lod: String, collision_enabled: bool):
	if _has_matching_chunk(chunk_coord, lod):
		var existing = get_chunk(chunk_coord)
		existing.set_collision_enabled(collision_enabled)
		return existing
	return _activate_chunk_sync(chunk_coord, lod, collision_enabled)

func _rebuild_pending(center_chunk: Vector2i, prewarm_center_chunk: Vector2i) -> void:
	var pending: Array[Dictionary] = []
	var desired_centers: Array[Vector2i] = [center_chunk]
	if prewarm_center_chunk != center_chunk:
		desired_centers.append(prewarm_center_chunk)
	var seen: Dictionary = {}
	for desired_center in desired_centers:
		for chunk_coord in _desired_chunk_order(desired_center, STREAM_RADIUS):
			var key := _chunk_key(chunk_coord)
			if seen.has(key):
				continue
			seen[key] = true
			var desired_lod := _desired_lod(center_chunk, chunk_coord)
			if _has_matching_chunk(chunk_coord, desired_lod):
				continue
			if _inflight_jobs.has(key):
				continue
			if chunk_coord == center_chunk:
				continue
			pending.append({
				"chunk_coord": chunk_coord,
				"lod": desired_lod,
				"collision_enabled": false,
			})
	_pending_requests = pending

func _desired_chunk_order(center_chunk: Vector2i, radius: int) -> Array[Vector2i]:
	var coords: Array[Vector2i] = [center_chunk]
	for ring in range(1, radius + 1):
		for dz in range(-ring, ring + 1):
			for dx in range(-ring, ring + 1):
				if maxi(absi(dx), absi(dz)) != ring:
					continue
				coords.append(center_chunk + Vector2i(dx, dz))
	return coords

func _desired_lod(_center_chunk: Vector2i, _chunk_coord: Vector2i) -> String:
	return ACTIVE_LOD

func _predict_next_center_chunk(center_chunk: Vector2i, player_position: Vector3, player_velocity: Vector3) -> Vector2i:
	var chunk_origin := GenerationManager.chunk_coord_to_scene_origin(center_chunk, _anchor_chunk)
	var local_x := player_position.x - chunk_origin.x
	var local_z := player_position.z - chunk_origin.z
	var abs_vx := absf(player_velocity.x)
	var abs_vz := absf(player_velocity.z)
	if abs_vx < PREWARM_MIN_SPEED and abs_vz < PREWARM_MIN_SPEED:
		return center_chunk
	if abs_vx >= abs_vz:
		if player_velocity.x > PREWARM_MIN_SPEED and local_x >= GenerationManager.BLOCKS_PER_CHUNK - PREWARM_EDGE_MARGIN:
			return center_chunk + Vector2i.RIGHT
		if player_velocity.x < -PREWARM_MIN_SPEED and local_x <= PREWARM_EDGE_MARGIN:
			return center_chunk + Vector2i.LEFT
	if abs_vz > abs_vx:
		if player_velocity.z > PREWARM_MIN_SPEED and local_z >= GenerationManager.BLOCKS_PER_CHUNK - PREWARM_EDGE_MARGIN:
			return center_chunk + Vector2i.DOWN
		if player_velocity.z < -PREWARM_MIN_SPEED and local_z <= PREWARM_EDGE_MARGIN:
			return center_chunk + Vector2i.UP
	return center_chunk

func _start_pending_jobs() -> void:
	while _inflight_jobs.size() < MAX_CONCURRENT_JOBS and not _pending_requests.is_empty():
		var request: Dictionary = _pending_requests[0]
		_pending_requests.remove_at(0)
		var chunk_coord: Vector2i = request["chunk_coord"]
		var lod := String(request["lod"])
		var config := GenerationManager.runtime_chunk_config_for_lod(lod)
		var world_origin := GenerationManager.chunk_coord_to_world_origin(chunk_coord)
		var job := MgChunkBuildJob.new()
		var started := job.start_chunk_build(
			GameState.world_seed,
			world_origin.x,
			world_origin.y,
			int(config["resolution"]),
			int(config["detail_level"]),
			float(config["freq_scale"]),
			VoxelMeshBuilder.HEIGHT_SCALE,
			int(config["sub_size"]),
			bool(config["use_edge_skirts"]),
		)
		if not started:
			continue
		var sample = _metrics.begin_activation(chunk_coord, lod)
		var key := _chunk_key(chunk_coord)
		var job_info := request.duplicate(true)
		job_info["job"] = job
		job_info["sample"] = sample
		_inflight_jobs[key] = job_info

func _attach_ready_jobs(center_chunk: Vector2i) -> void:
	var ready_keys: Array[String] = []
	for key in _inflight_jobs.keys():
		var job_info: Dictionary = _inflight_jobs[key]
		var job: MgChunkBuildJob = job_info["job"]
		if job.is_ready():
			ready_keys.append(String(key))
	if ready_keys.is_empty():
		return

	var attached := 0
	for key in ready_keys:
		if attached >= MAX_READY_ATTACHES_PER_FRAME:
			break
		var job_info: Dictionary = _inflight_jobs[key]
		_inflight_jobs.erase(key)
		var chunk_coord: Vector2i = job_info["chunk_coord"]
		var lod := String(job_info["lod"])
		var collision_enabled := bool(job_info["collision_enabled"])
		var sample: Dictionary = job_info["sample"]
		var job: MgChunkBuildJob = job_info["job"]

		if not _is_within_retain_radius(chunk_coord, center_chunk):
			continue
		if _has_matching_chunk(chunk_coord, lod):
			continue

		var result := job.take_result()
		if result.is_empty():
			continue
		_metrics.set_phase_ms(
			sample,
			"generation_ms",
			float(result.get("generation_ms", 0.0)) + float(result.get("mesh_prep_ms", 0.0))
		)

		var world_chunk = WorldChunkScript.new()
		world_chunk.configure(chunk_coord, _anchor_chunk, lod)
		var build_result := world_chunk.activate_from_chunk_data(
			result["biome_map"],
			result["mesh_data"],
			_builder,
			collision_enabled,
		)
		_metrics.set_phase_ms(sample, "mesh_ms", float(build_result["mesh_ms"]))
		_metrics.set_phase_ms(sample, "collision_ms", float(build_result["collision_ms"]))

		var attach_start := Time.get_ticks_usec()
		_terrain_root.add_child(world_chunk)
		_metrics.set_phase_ms(sample, "attach_ms", (Time.get_ticks_usec() - attach_start) / 1000.0)

		_chunks[key] = world_chunk
		_metrics.finish_activation(sample)
		attached += 1

func _update_collision_focus(center_chunk: Vector2i) -> void:
	for chunk in _chunks.values():
		chunk.set_collision_enabled(_is_within_collision_radius(chunk.chunk_coord, center_chunk))

func _unload_distant_chunks(center_chunk: Vector2i) -> void:
	var unload_keys: Array[String] = []
	for key in _chunks.keys():
		var chunk = _chunks[key]
		if _is_within_retain_radius(chunk.chunk_coord, center_chunk):
			continue
		unload_keys.append(String(key))

	for key in unload_keys:
		var chunk = _chunks[key]
		chunk.begin_unload()
		_chunks.erase(key)
		chunk.queue_free()

func _has_matching_chunk(chunk_coord: Vector2i, lod: String) -> bool:
	var chunk = get_chunk(chunk_coord)
	if chunk == null:
		return false
	return String(chunk.lod) == lod

func _is_within_retain_radius(chunk_coord: Vector2i, center_chunk: Vector2i) -> bool:
	var dx := absi(chunk_coord.x - center_chunk.x)
	var dy := absi(chunk_coord.y - center_chunk.y)
	return maxi(dx, dy) <= RETAIN_RADIUS

func _is_within_collision_radius(chunk_coord: Vector2i, center_chunk: Vector2i) -> bool:
	var dx := absi(chunk_coord.x - center_chunk.x)
	var dy := absi(chunk_coord.y - center_chunk.y)
	return maxi(dx, dy) <= COLLISION_RADIUS

func _chunk_key(chunk_coord: Vector2i) -> String:
	return "%d,%d" % [chunk_coord.x, chunk_coord.y]
