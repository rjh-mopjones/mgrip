extends RefCounted
class_name AgentActionValidator

const DEFAULT_MOVE_SPEED := 12.0
const DEFAULT_MOVE_DURATION := 1.0
const DEFAULT_MOVE_TIMEOUT := 2.5
const DEFAULT_MOVE_TO_TIMEOUT := 10.0
const DEFAULT_STOP_TIMEOUT := 1.0
const DEFAULT_WAIT_FOR_CHUNK_TIMEOUT := 8.0
const DEFAULT_WAIT_FOR_RING_TIMEOUT := 10.0
const DEFAULT_WAIT_FOR_SETTLE_TIMEOUT := 3.0
const DEFAULT_SETTLE_STABLE_DURATION := 0.15

func validate_action(action_name: String, params: Dictionary) -> Dictionary:
	var normalized_name := action_name.strip_edges()
	if normalized_name.is_empty():
		return _reject("missing_action_name", "Action name is required.", "Provide one of the supported agent action names.")

	match normalized_name:
		"teleport_to_block":
			return _validate_teleport(params)
		"look_at":
			return _validate_look_at(params)
		"move_in_direction":
			return _validate_move_in_direction(params)
		"move_to_block":
			return _validate_move_to_block(params)
		"stop":
			return _validate_stop(params)
		"sample_height":
			return _validate_scene_block_query(normalized_name, params)
		"find_nearest_land":
			return _validate_scene_block_query(normalized_name, params)
		"wait_seconds":
			return _validate_wait(params)
		"capture_screenshot":
			return _validate_capture_screenshot(params)
		"get_chunk_state":
			return _validate_get_chunk_state(params)
		"wait_for_chunk_loaded":
			return _validate_wait_for_chunk_loaded(params)
		"wait_for_ring_ready":
			return _validate_wait_for_ring_ready(params)
		"wait_for_player_settled":
			return _validate_wait_for_player_settled(params)
		"end_session":
			return {"ok": true, "action": {"name": normalized_name, "params": {}}}
		_:
			return _reject("unsupported_action", "Unsupported agent action: %s" % normalized_name, "Use one of the actions listed in specs/004-agent-playtest-runtime.md.")

func _validate_teleport(params: Dictionary) -> Dictionary:
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	if scene_block == null:
		return _reject("missing_scene_block", "`teleport_to_block` requires a `scene_block` target.", "Pass `scene_block` as a Vector2i, Vector2, or `{x, z}` dictionary.")
	return {
		"ok": true,
		"action": {
			"name": "teleport_to_block",
			"params": {
				"scene_block": scene_block,
				"height_offset": float(params.get("height_offset", 3.0)),
			},
		},
	}

func _validate_look_at(params: Dictionary) -> Dictionary:
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	var target_point = coerce_point3(params.get("target_point", null))
	if scene_block == null and target_point == null:
		return _reject("missing_look_target", "`look_at` requires either `scene_block` or `target_point`.", "Use a scene-space block target for terrain lookups or a full 3D point.")
	return {
		"ok": true,
		"action": {
			"name": "look_at",
			"params": {
				"scene_block": scene_block,
				"target_point": target_point,
				"height_offset": float(params.get("height_offset", 0.0)),
				"tolerance_degrees": maxf(0.1, float(params.get("tolerance_degrees", 1.0))),
			},
		},
	}

func _validate_move_in_direction(params: Dictionary) -> Dictionary:
	var direction = coerce_direction3(params.get("direction", null))
	if direction == null or direction.is_zero_approx():
		return _reject("invalid_direction", "`move_in_direction` requires a non-zero horizontal direction.", "Provide `direction` as a Vector2, Vector3, or `{x, z}` dictionary.")
	var duration_seconds := maxf(0.05, float(params.get("duration_seconds", DEFAULT_MOVE_DURATION)))
	var timeout_seconds := maxf(duration_seconds, float(params.get("timeout_seconds", DEFAULT_MOVE_TIMEOUT)))
	return {
		"ok": true,
		"action": {
			"name": "move_in_direction",
			"params": {
				"direction": direction,
				"speed": maxf(0.1, float(params.get("speed", DEFAULT_MOVE_SPEED))),
				"duration_seconds": duration_seconds,
				"timeout_seconds": timeout_seconds,
				"stop_tolerance": maxf(0.01, float(params.get("stop_tolerance", 0.25))),
			},
		},
	}

func _validate_move_to_block(params: Dictionary) -> Dictionary:
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	if scene_block == null:
		return _reject("missing_scene_block", "`move_to_block` requires a `scene_block` target.", "Pass `scene_block` as a Vector2i, Vector2, or `{x, z}` dictionary.")
	return {
		"ok": true,
		"action": {
			"name": "move_to_block",
			"params": {
				"scene_block": scene_block,
				"speed": maxf(0.1, float(params.get("speed", DEFAULT_MOVE_SPEED))),
				"arrival_radius": maxf(0.2, float(params.get("arrival_radius", 1.25))),
				"timeout_seconds": maxf(0.5, float(params.get("timeout_seconds", DEFAULT_MOVE_TO_TIMEOUT))),
				"stop_tolerance": maxf(0.01, float(params.get("stop_tolerance", 0.25))),
			},
		},
	}

func _validate_stop(params: Dictionary) -> Dictionary:
	return {
		"ok": true,
		"action": {
			"name": "stop",
			"params": {
				"timeout_seconds": maxf(0.1, float(params.get("timeout_seconds", DEFAULT_STOP_TIMEOUT))),
				"stop_tolerance": maxf(0.01, float(params.get("stop_tolerance", 0.25))),
			},
		},
	}

func _validate_scene_block_query(action_name: String, params: Dictionary) -> Dictionary:
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	if scene_block == null:
		return _reject("missing_scene_block", "`%s` requires a `scene_block` target." % action_name, "Pass `scene_block` as a Vector2i, Vector2, or `{x, z}` dictionary.")
	return {
		"ok": true,
		"action": {
			"name": action_name,
			"params": {
				"scene_block": scene_block,
			},
		},
	}

func _validate_wait(params: Dictionary) -> Dictionary:
	var duration_seconds := float(params.get("duration_seconds", params.get("seconds", 0.0)))
	if duration_seconds <= 0.0:
		return _reject("invalid_duration", "`wait_seconds` requires a positive duration.", "Provide `duration_seconds` greater than zero.")
	return {
		"ok": true,
		"action": {
			"name": "wait_seconds",
			"params": {
				"duration_seconds": duration_seconds,
				"timeout_seconds": maxf(duration_seconds, float(params.get("timeout_seconds", duration_seconds + 0.5))),
			},
		},
	}

func _validate_capture_screenshot(params: Dictionary) -> Dictionary:
	return {
		"ok": true,
		"action": {
			"name": "capture_screenshot",
			"params": {
				"file_name": String(params.get("file_name", "capture")),
				"path": String(params.get("path", "")),
			},
		},
	}

func _validate_get_chunk_state(params: Dictionary) -> Dictionary:
	var chunk_coord = coerce_chunk_coord(params.get("chunk_coord", null))
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	if chunk_coord == null and scene_block == null:
		return {
			"ok": true,
			"action": {
				"name": "get_chunk_state",
				"params": {},
			},
		}
	return {
		"ok": true,
		"action": {
			"name": "get_chunk_state",
			"params": {
				"chunk_coord": chunk_coord,
				"scene_block": scene_block,
			},
		},
	}

func _validate_wait_for_chunk_loaded(params: Dictionary) -> Dictionary:
	var chunk_coord = coerce_chunk_coord(params.get("chunk_coord", null))
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	var required_lod := String(params.get("required_lod", "")).strip_edges()
	return {
		"ok": true,
		"action": {
			"name": "wait_for_chunk_loaded",
			"params": {
				"chunk_coord": chunk_coord,
				"scene_block": scene_block,
				"required_lod": required_lod,
				"timeout_seconds": maxf(0.1, float(params.get("timeout_seconds", DEFAULT_WAIT_FOR_CHUNK_TIMEOUT))),
			},
		},
	}

func _validate_wait_for_ring_ready(params: Dictionary) -> Dictionary:
	var chunk_coord = coerce_chunk_coord(params.get("chunk_coord", null))
	var scene_block = coerce_scene_block(params.get("scene_block", null))
	return {
		"ok": true,
		"action": {
			"name": "wait_for_ring_ready",
			"params": {
				"chunk_coord": chunk_coord,
				"scene_block": scene_block,
				"radius": maxi(0, int(params.get("radius", 1))),
				"timeout_seconds": maxf(0.1, float(params.get("timeout_seconds", DEFAULT_WAIT_FOR_RING_TIMEOUT))),
			},
		},
	}

func _validate_wait_for_player_settled(params: Dictionary) -> Dictionary:
	return {
		"ok": true,
		"action": {
			"name": "wait_for_player_settled",
			"params": {
				"horizontal_speed_tolerance": maxf(0.01, float(params.get("horizontal_speed_tolerance", params.get("stop_tolerance", 0.25)))),
				"vertical_speed_tolerance": maxf(0.01, float(params.get("vertical_speed_tolerance", 0.5))),
				"stable_duration_seconds": maxf(0.0, float(params.get("stable_duration_seconds", DEFAULT_SETTLE_STABLE_DURATION))),
				"require_on_floor": bool(params.get("require_on_floor", true)),
				"timeout_seconds": maxf(0.1, float(params.get("timeout_seconds", DEFAULT_WAIT_FOR_SETTLE_TIMEOUT))),
			},
		},
	}

func _reject(error_code: String, reason: String, hint: String = "") -> Dictionary:
	return {
		"ok": false,
		"error_code": error_code,
		"reason": reason,
		"hint": hint,
	}

static func coerce_scene_block(value):
	if value is Vector2i:
		return value
	if value is Vector2:
		return Vector2i(int(round(value.x)), int(round(value.y)))
	if value is Dictionary:
		if value.has("x") and value.has("z"):
			return Vector2i(int(round(float(value["x"]))), int(round(float(value["z"]))))
		if value.has("x") and value.has("y"):
			return Vector2i(int(round(float(value["x"]))), int(round(float(value["y"]))))
	if value is Array and value.size() >= 2:
		return Vector2i(int(round(float(value[0]))), int(round(float(value[1]))))
	return null

static func coerce_chunk_coord(value):
	if value is Vector2i:
		return value
	if value is Vector2:
		return Vector2i(int(round(value.x)), int(round(value.y)))
	if value is Dictionary and value.has("x") and value.has("y"):
		return Vector2i(int(round(float(value["x"]))), int(round(float(value["y"]))))
	if value is Array and value.size() >= 2:
		return Vector2i(int(round(float(value[0]))), int(round(float(value[1]))))
	return null

static func coerce_point3(value):
	if value is Vector3:
		return value
	if value is Dictionary:
		if value.has("x") and value.has("y") and value.has("z"):
			return Vector3(float(value["x"]), float(value["y"]), float(value["z"]))
	if value is Array and value.size() >= 3:
		return Vector3(float(value[0]), float(value[1]), float(value[2]))
	return null

static func coerce_direction3(value):
	if value is Vector3:
		return Vector3(value.x, 0.0, value.z).normalized()
	if value is Vector2:
		return Vector3(value.x, 0.0, value.y).normalized()
	if value is Dictionary:
		if value.has("x") and value.has("z"):
			return Vector3(float(value["x"]), 0.0, float(value["z"])).normalized()
		if value.has("x") and value.has("y"):
			return Vector3(float(value["x"]), 0.0, float(value["y"])).normalized()
	if value is Array and value.size() >= 2:
		return Vector3(float(value[0]), 0.0, float(value[1])).normalized()
	return null
