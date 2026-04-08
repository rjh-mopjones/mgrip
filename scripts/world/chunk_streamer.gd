extends RefCounted
class_name ChunkStreamer

const ACTIVE_LOD := GenerationManager.LOD0_NAME
const STREAM_RADIUS := 1
const MID_HORIZON_LOD := GenerationManager.LOD1_NAME
const FAR_HORIZON_LOD := GenerationManager.LOD2_NAME
const DEFAULT_MID_HORIZON_RADIUS := 3
const FLIGHT_MID_HORIZON_RADIUS := 5
const DEFAULT_FAR_HORIZON_RADIUS := 5
const FLIGHT_FAR_HORIZON_RADIUS := 8
const DEFAULT_MID_HORIZON_BUDGET := 40
const FLIGHT_MID_HORIZON_BUDGET := 72
const DEFAULT_FAR_HORIZON_BUDGET := 32
const FLIGHT_FAR_HORIZON_BUDGET := 96
const COLLISION_RADIUS := 1
const PREWARM_EDGE_MARGIN := 72.0
const PREWARM_MIN_SPEED := 2.0
const FOCUS_DIRECTION_MIN_SPEED := 4.0
const DIRECTION_PRIORITY_WEIGHT := 6.0
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
var _mid_horizon_radius := DEFAULT_MID_HORIZON_RADIUS
var _far_horizon_radius := DEFAULT_FAR_HORIZON_RADIUS
var _mid_horizon_budget := DEFAULT_MID_HORIZON_BUDGET
var _far_horizon_budget := DEFAULT_FAR_HORIZON_BUDGET
var _retain_radius := DEFAULT_FAR_HORIZON_RADIUS + 1
var _focus_direction := Vector2.ZERO

func setup(terrain_root: Node3D, metrics, anchor_chunk: Vector2i) -> void:
	_terrain_root = terrain_root
	_metrics = metrics
	_anchor_chunk = anchor_chunk
	_current_center_chunk = anchor_chunk
	_mid_horizon_radius = FLIGHT_MID_HORIZON_RADIUS if _is_flight_mode() else DEFAULT_MID_HORIZON_RADIUS
	_far_horizon_radius = FLIGHT_FAR_HORIZON_RADIUS if _is_flight_mode() else DEFAULT_FAR_HORIZON_RADIUS
	_mid_horizon_budget = FLIGHT_MID_HORIZON_BUDGET if _is_flight_mode() else DEFAULT_MID_HORIZON_BUDGET
	_far_horizon_budget = FLIGHT_FAR_HORIZON_BUDGET if _is_flight_mode() else DEFAULT_FAR_HORIZON_BUDGET
	_retain_radius = _far_horizon_radius + 1

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

func update_streaming(
		center_chunk: Vector2i,
		player_position: Vector3 = Vector3.ZERO,
		player_velocity: Vector3 = Vector3.ZERO,
		player_forward: Vector3 = Vector3.FORWARD) -> void:
	_current_center_chunk = center_chunk
	_prewarm_target_chunk = _predict_next_center_chunk(center_chunk, player_position, player_velocity)
	_focus_direction = _resolve_focus_direction(player_velocity, player_forward)
	var desired_lods := _collect_desired_lods(center_chunk, _prewarm_target_chunk, _focus_direction)
	_attach_ready_jobs(center_chunk, desired_lods)
	_ensure_chunk_sync(center_chunk, ACTIVE_LOD, true)
	_rebuild_pending(center_chunk, _prewarm_target_chunk, _focus_direction, desired_lods)
	_start_pending_jobs()
	_update_collision_focus(center_chunk)
	_unload_obsolete_chunks(center_chunk, desired_lods)
	_metrics.update_runtime_state(active_counts_by_lod(), pending_count())
	_metrics.set_horizon_state(horizon_runtime_state())

func prewarm_target_chunk() -> Vector2i:
	return _prewarm_target_chunk

func horizon_runtime_state() -> Dictionary:
	return {
		"focus": _focus_direction,
		"mid_radius": _mid_horizon_radius,
		"mid_budget": _mid_horizon_budget,
		"mid_loaded": _loaded_count_for_lod(MID_HORIZON_LOD),
		"far_radius": _far_horizon_radius,
		"far_budget": _far_horizon_budget,
		"far_loaded": _loaded_count_for_lod(FAR_HORIZON_LOD),
	}

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

func _rebuild_pending(
		center_chunk: Vector2i,
		prewarm_center_chunk: Vector2i,
		focus_direction: Vector2,
		desired_lods: Dictionary) -> void:
	var pending: Array[Dictionary] = []
	for chunk_coord in _ordered_desired_chunks(center_chunk, prewarm_center_chunk, focus_direction):
		var key := _chunk_key(chunk_coord)
		var desired_lod := String(desired_lods.get(key, ""))
		if desired_lod.is_empty():
			continue
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

func _collect_desired_lods(center_chunk: Vector2i, prewarm_center_chunk: Vector2i, focus_direction: Vector2) -> Dictionary:
	var desired: Dictionary = {}
	for chunk_coord in _desired_chunk_order(center_chunk, STREAM_RADIUS):
		_set_desired_lod(desired, chunk_coord, ACTIVE_LOD)
	for chunk_coord in _budgeted_horizon_chunks(
		center_chunk,
		STREAM_RADIUS + 1,
		_mid_horizon_radius,
		_mid_horizon_budget,
		focus_direction
	):
		_set_desired_lod(desired, chunk_coord, MID_HORIZON_LOD)
	for chunk_coord in _budgeted_horizon_chunks(
		center_chunk,
		_mid_horizon_radius + 1,
		_far_horizon_radius,
		_far_horizon_budget,
		focus_direction
	):
		_set_desired_lod(desired, chunk_coord, FAR_HORIZON_LOD)
	if prewarm_center_chunk != center_chunk:
		for chunk_coord in _desired_chunk_order(prewarm_center_chunk, STREAM_RADIUS):
			_set_desired_lod(desired, chunk_coord, _desired_lod_for_center(prewarm_center_chunk, chunk_coord, true))
	return desired

func _ordered_desired_chunks(center_chunk: Vector2i, prewarm_center_chunk: Vector2i, focus_direction: Vector2) -> Array[Vector2i]:
	var ordered: Array[Vector2i] = []
	var seen: Dictionary = {}
	if prewarm_center_chunk != center_chunk:
		for chunk_coord in _desired_chunk_order(prewarm_center_chunk, STREAM_RADIUS):
			var key := _chunk_key(chunk_coord)
			if seen.has(key):
				continue
			seen[key] = true
			ordered.append(chunk_coord)
	for chunk_coord in _desired_chunk_order(center_chunk, STREAM_RADIUS):
		var center_key := _chunk_key(chunk_coord)
		if seen.has(center_key):
			continue
		seen[center_key] = true
		ordered.append(chunk_coord)
	for chunk_coord in _budgeted_horizon_chunks(
		center_chunk,
		STREAM_RADIUS + 1,
		_mid_horizon_radius,
		_mid_horizon_budget,
		focus_direction
	):
		var key := _chunk_key(chunk_coord)
		if seen.has(key):
			continue
		seen[key] = true
		ordered.append(chunk_coord)
	for chunk_coord in _budgeted_horizon_chunks(
		center_chunk,
		_mid_horizon_radius + 1,
		_far_horizon_radius,
		_far_horizon_budget,
		focus_direction
	):
		var far_key := _chunk_key(chunk_coord)
		if seen.has(far_key):
			continue
		seen[far_key] = true
		ordered.append(chunk_coord)
	return ordered

func _desired_chunk_order(center_chunk: Vector2i, radius: int) -> Array[Vector2i]:
	var coords: Array[Vector2i] = [center_chunk]
	for ring in range(1, radius + 1):
		for dz in range(-ring, ring + 1):
			for dx in range(-ring, ring + 1):
				if maxi(absi(dx), absi(dz)) != ring:
					continue
				coords.append(center_chunk + Vector2i(dx, dz))
	return coords

func _desired_lod_for_center(center_chunk: Vector2i, chunk_coord: Vector2i, near_only: bool) -> String:
	var dx := absi(chunk_coord.x - center_chunk.x)
	var dy := absi(chunk_coord.y - center_chunk.y)
	var radius := maxi(dx, dy)
	if radius <= STREAM_RADIUS:
		return ACTIVE_LOD
	if near_only:
		return ""
	if radius <= _mid_horizon_radius:
		return MID_HORIZON_LOD
	if radius <= _far_horizon_radius:
		return FAR_HORIZON_LOD
	return ""

func _budgeted_horizon_chunks(
		center_chunk: Vector2i,
		min_radius: int,
		max_radius: int,
		budget: int,
		focus_direction: Vector2) -> Array[Vector2i]:
	var selected: Array[Dictionary] = []
	var candidate_count := 0
	for chunk_coord in _desired_chunk_order(center_chunk, max_radius):
		var dx := absi(chunk_coord.x - center_chunk.x)
		var dy := absi(chunk_coord.y - center_chunk.y)
		var radius := maxi(dx, dy)
		if radius < min_radius or radius > max_radius:
			continue
		candidate_count += 1
		var candidate := {
			"chunk_coord": chunk_coord,
			"score": _candidate_priority_score(center_chunk, chunk_coord, focus_direction),
		}
		_insert_ranked_candidate(selected, candidate, budget)
	if budget <= 0 or candidate_count <= budget:
		var all_chunks: Array[Vector2i] = []
		for item in selected:
			all_chunks.append(item["chunk_coord"])
		return all_chunks
	var chunks: Array[Vector2i] = []
	for item in selected:
		chunks.append(item["chunk_coord"])
	return chunks

func _insert_ranked_candidate(selected: Array[Dictionary], candidate: Dictionary, budget: int) -> void:
	if budget <= 0:
		return
	var inserted := false
	for i in range(selected.size()):
		if float(candidate["score"]) > float(selected[i]["score"]):
			selected.insert(i, candidate)
			inserted = true
			break
	if not inserted:
		selected.append(candidate)
	if selected.size() > budget:
		selected.resize(budget)

func _candidate_priority_score(center_chunk: Vector2i, chunk_coord: Vector2i, focus_direction: Vector2) -> float:
	var delta := chunk_coord - center_chunk
	var radius := float(maxi(absi(delta.x), absi(delta.y)))
	var distance_score := -radius * 100.0
	var manhattan_score := -float(absi(delta.x) + absi(delta.y))
	var alignment_score := _direction_alignment(delta, focus_direction) * DIRECTION_PRIORITY_WEIGHT
	return distance_score + manhattan_score + alignment_score

func _direction_alignment(delta: Vector2i, focus_direction: Vector2) -> float:
	if focus_direction.length_squared() <= 0.0001:
		return 0.0
	var chunk_dir := Vector2(float(delta.x), float(delta.y))
	if chunk_dir.length_squared() <= 0.0001:
		return 0.0
	return maxf(0.0, chunk_dir.normalized().dot(focus_direction))

func _set_desired_lod(desired: Dictionary, chunk_coord: Vector2i, lod: String) -> void:
	if lod.is_empty():
		return
	var key := _chunk_key(chunk_coord)
	var existing := String(desired.get(key, ""))
	if existing == ACTIVE_LOD:
		return
	if lod == ACTIVE_LOD or existing.is_empty():
		desired[key] = lod

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

func _resolve_focus_direction(player_velocity: Vector3, player_forward: Vector3) -> Vector2:
	var planar_velocity := Vector2(player_velocity.x, player_velocity.z)
	if planar_velocity.length() >= FOCUS_DIRECTION_MIN_SPEED:
		return planar_velocity.normalized()
	var planar_forward := Vector2(player_forward.x, player_forward.z)
	if planar_forward.length_squared() > 0.0001:
		return planar_forward.normalized()
	return Vector2.ZERO

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

func _attach_ready_jobs(center_chunk: Vector2i, desired_lods: Dictionary) -> void:
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
		var desired_lod := String(desired_lods.get(key, ""))

		if desired_lod.is_empty():
			continue
		if not _is_within_retain_radius(chunk_coord, center_chunk):
			continue
		if lod != desired_lod:
			continue
		if _has_matching_chunk(chunk_coord, desired_lod):
			continue
		if _chunks.has(key):
			var existing = _chunks[key]
			existing.begin_unload()
			_chunks.erase(key)
			existing.queue_free()

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

func _unload_obsolete_chunks(center_chunk: Vector2i, desired_lods: Dictionary) -> void:
	var unload_keys: Array[String] = []
	for key in _chunks.keys():
		var chunk = _chunks[key]
		var desired_lod := String(desired_lods.get(String(key), ""))
		if desired_lod.is_empty():
			if _is_within_retain_radius(chunk.chunk_coord, center_chunk):
				unload_keys.append(String(key))
				continue
			unload_keys.append(String(key))
			continue
		if String(chunk.lod) != desired_lod:
			unload_keys.append(String(key))
			continue
		if not _is_within_retain_radius(chunk.chunk_coord, center_chunk):
			unload_keys.append(String(key))
			continue

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
	return maxi(dx, dy) <= _retain_radius

func _is_within_collision_radius(chunk_coord: Vector2i, center_chunk: Vector2i) -> bool:
	var dx := absi(chunk_coord.x - center_chunk.x)
	var dy := absi(chunk_coord.y - center_chunk.y)
	return maxi(dx, dy) <= COLLISION_RADIUS

func _chunk_key(chunk_coord: Vector2i) -> String:
	return "%d,%d" % [chunk_coord.x, chunk_coord.y]

func _loaded_count_for_lod(lod: String) -> int:
	var count := 0
	for chunk in _chunks.values():
		if String(chunk.lod) == lod:
			count += 1
	return count

func _is_flight_mode() -> bool:
	for arg in OS.get_cmdline_args():
		var value := String(arg)
		if value == "--flythrough-flight" or value == "--flythrough=flight":
			return true
	return false
