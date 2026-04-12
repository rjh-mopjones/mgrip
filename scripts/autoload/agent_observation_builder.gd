extends RefCounted
class_name AgentObservationBuilder

const AgentSessionScript = preload("res://scripts/autoload/agent_session.gd")
const AgentActionValidatorScript = preload("res://scripts/autoload/agent_action_validator.gd")
const DEFAULT_HEIGHT_SAMPLE_OFFSETS: Array[Vector2i] = [
	Vector2i.ZERO,
	Vector2i(16, 0),
	Vector2i(-16, 0),
	Vector2i(0, 16),
	Vector2i(0, -16),
]

## Five-point cross around the player's current chunk for macro observation.
## Offset in world units (chunks). Sampling a cross rather than a full 3x3
## grid keeps the observation payload small while still covering the four
## cardinal neighbors — enough for an agent to see if it's approaching a
## coastline, river, or climate band in any direction.
const MACRO_SAMPLE_OFFSETS: Array[Vector2] = [
	Vector2.ZERO,
	Vector2(1.0, 0.0),
	Vector2(-1.0, 0.0),
	Vector2(0.0, 1.0),
	Vector2(0.0, -1.0),
]

func build(
		world,
		player: CharacterBody3D,
		head: Node3D,
		camera: Camera3D,
		chunk_streamer,
		session,
		options: Dictionary = {}) -> Dictionary:
	var timestamp_ms := Time.get_ticks_msec()
	var base := {
		"schema_version": 2,
		"session_id": session.session_id if session != null else "",
		"step_index": session.step_count if session != null else 0,
		"timestamp_ms": timestamp_ms,
		"runtime_available": world != null and player != null and chunk_streamer != null,
	}
	if world == null or player == null or chunk_streamer == null:
		base["error_code"] = "no_world_runtime"
		base["reason"] = "World runtime is not currently registered with AgentRuntime."
		return base

	var player_forward := _resolve_forward_vector(camera, head, player)
	var probe_points: Array = options.get("probe_points", [])
	base["world_seed"] = GameState.world_seed
	base["display_driver"] = DisplayServer.get_name()
	base["runtime_constants"] = {
		"blocks_per_chunk": GenerationManager.BLOCKS_PER_CHUNK,
		"world_units_per_chunk": GenerationManager.WORLD_UNITS_PER_CHUNK,
	}
	base["launch"] = {
		"mode": GameState.runtime_launch_mode_name(),
		"world_origin": AgentSessionScript.sanitize_variant(GameState.runtime_launch_world_origin),
		"chunk_coord": AgentSessionScript.sanitize_variant(GameState.runtime_launch_chunk),
	}
	base["anchor_chunk"] = AgentSessionScript.vector2i_to_dict(GameState.anchor_chunk)
	base["current_chunk"] = AgentSessionScript.vector2i_to_dict(GameState.current_chunk)
	base["player_position"] = AgentSessionScript.vector3_to_dict(player.position)
	base["player_velocity"] = AgentSessionScript.vector3_to_dict(player.velocity)
	base["player_facing_direction"] = AgentSessionScript.vector3_to_dict(player_forward)
	base["current_loaded_chunk_counts_by_lod"] = AgentSessionScript.sanitize_variant(chunk_streamer.active_counts_by_lod())
	base["pending_chunk_job_count"] = chunk_streamer.pending_count()
	base["prewarm_target_chunk"] = AgentSessionScript.vector2i_to_dict(chunk_streamer.prewarm_target_chunk())
	base["nearby_sampled_terrain_heights"] = _sample_nearby_heights(world, player.position)
	base["nearest_land_results"] = _nearest_land_results(world, probe_points)
	base["macro_semantics_sample"] = _sample_macro_semantics_ring(world)
	base["flythrough"] = _flythrough_state()
	var current_chunk_state: Dictionary = AgentSessionScript.sanitize_variant(
		world.get_current_chunk_state() if world.has_method("get_current_chunk_state") else {}
	)
	base["current_chunk_state"] = current_chunk_state
	base["runtime_presentation"] = AgentSessionScript.sanitize_variant(
		world.get_current_runtime_presentation() if world.has_method("get_current_runtime_presentation")
		else current_chunk_state.get("runtime_presentation", {})
	)
	if bool(options.get("debug", false)):
		base["debug"] = {
			"horizon_runtime_state": AgentSessionScript.sanitize_variant(chunk_streamer.horizon_runtime_state()),
			"ring_ready": chunk_streamer.is_ring_ready(GameState.current_chunk),
			"collision_enabled_chunk_set": AgentSessionScript.sanitize_variant(
				chunk_streamer.collision_enabled_chunk_coords(GameState.current_chunk)
			),
		}
	return base

func _sample_macro_semantics_ring(world) -> Array[Dictionary]:
	# Sample macro truth at the player's current chunk plus a cardinal cross
	# of neighbors. Each chunk is 1 world unit, so offsets are in world units.
	# Skipping entries where macro is unloaded (loaded == false) keeps the
	# observation compact — agent callers see an empty array and branch.
	if world == null or not world.has_method("sample_macro_semantics"):
		return []
	var samples: Array[Dictionary] = []
	var anchor := Vector2(GameState.current_chunk.x, GameState.current_chunk.y)
	for offset in MACRO_SAMPLE_OFFSETS:
		var world_point := anchor + offset + Vector2(0.5, 0.5)
		var sample: Dictionary = world.sample_macro_semantics(world_point.x, world_point.y)
		if sample.is_empty() or not sample.get("loaded", false):
			continue
		sample["sample_world"] = {"x": world_point.x, "y": world_point.y}
		sample["sample_chunk"] = {
			"x": int(floor(world_point.x)),
			"y": int(floor(world_point.y)),
		}
		samples.append(AgentSessionScript.sanitize_variant(sample))
	return samples


func _sample_nearby_heights(world, player_position: Vector3) -> Array[Dictionary]:
	var center_block := Vector2i(
		int(round(player_position.x - 0.5)),
		int(round(player_position.z - 0.5))
	)
	var samples: Array[Dictionary] = []
	for offset in DEFAULT_HEIGHT_SAMPLE_OFFSETS:
		var sample_block := center_block + offset
		samples.append({
			"scene_block": {
				"x": sample_block.x,
				"z": sample_block.y,
			},
			"height": float(world.sample_surface_height(sample_block.x, sample_block.y)),
		})
	return samples

func _nearest_land_results(world, probe_points: Array) -> Array[Dictionary]:
	var results: Array[Dictionary] = []
	for point in probe_points:
		var scene_block = AgentActionValidatorScript.coerce_scene_block(point)
		if scene_block == null:
			continue
		var nearest_land: Vector2 = world.nearest_land_block(scene_block.x, scene_block.y)
		results.append({
			"requested_scene_block": {
				"x": scene_block.x,
				"z": scene_block.y,
			},
			"nearest_land_scene_block": {
				"x": int(round(nearest_land.x)),
				"z": int(round(nearest_land.y)),
			},
			"height": float(world.sample_surface_height(int(round(nearest_land.x)), int(round(nearest_land.y)))),
		})
	return results

func _resolve_forward_vector(camera: Camera3D, head: Node3D, player: CharacterBody3D) -> Vector3:
	if camera != null:
		return -camera.global_transform.basis.z
	if head != null:
		return -head.global_transform.basis.z
	return -player.global_transform.basis.z

func _flythrough_state() -> Dictionary:
	var flythrough = Engine.get_main_loop().root.get_node_or_null("Flythrough")
	if flythrough == null:
		return {
			"active": false,
			"mode": "",
		}
	return {
		"active": bool(flythrough.call("is_active")) if flythrough.has_method("is_active") else false,
		"mode": String(flythrough.call("current_mode")) if flythrough.has_method("current_mode") else "",
	}
