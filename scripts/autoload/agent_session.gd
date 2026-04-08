extends RefCounted
class_name AgentSession

var session_id: String = ""
var scenario_label: String = ""
var status: String = "inactive"
var started_at_ms: int = 0
var ended_at_ms: int = 0
var step_count: int = 0
var seed: int = 0
var anchor_chunk: Vector2i = Vector2i.ZERO
var current_chunk: Vector2i = Vector2i.ZERO
var player_position: Vector3 = Vector3.ZERO
var last_action: Dictionary = {}
var last_result: Dictionary = {}
var artifact_dir: String = ""
var session_file_path: String = ""
var steps_file_path: String = ""
var screenshots_dir: String = ""
var metadata: Dictionary = {}

func configure(goal_label: String, initial_observation: Dictionary, extra_metadata: Dictionary = {}) -> void:
	started_at_ms = Time.get_ticks_msec()
	scenario_label = goal_label
	status = "active"
	seed = int(initial_observation.get("world_seed", GameState.world_seed))
	anchor_chunk = _dict_to_vector2i(initial_observation.get("anchor_chunk", Vector2i.ZERO))
	current_chunk = _dict_to_vector2i(initial_observation.get("current_chunk", Vector2i.ZERO))
	player_position = _dict_to_vector3(initial_observation.get("player_position", Vector3.ZERO))
	session_id = "session_%d_%d" % [int(Time.get_unix_time_from_system()), Time.get_ticks_usec()]
	artifact_dir = "user://agent_sessions/%s" % session_id
	session_file_path = "%s/session.json" % artifact_dir
	steps_file_path = "%s/steps.jsonl" % artifact_dir
	screenshots_dir = "%s/screenshots" % artifact_dir
	metadata = extra_metadata.duplicate(true)
	metadata["session_id"] = session_id
	metadata["scenario_label"] = scenario_label
	metadata["transport"] = "local_runtime_api"
	metadata["artifact_dir"] = artifact_dir
	metadata["artifact_dir_absolute"] = ProjectSettings.globalize_path(artifact_dir)
	metadata["screenshots_dir"] = screenshots_dir
	metadata["screenshots_dir_absolute"] = ProjectSettings.globalize_path(screenshots_dir)
	metadata["started_at_ms"] = started_at_ms
	metadata["initial_observation"] = sanitize_variant(initial_observation)
	_ensure_artifact_dirs()
	_write_text_file(steps_file_path, "")
	persist_summary(initial_observation)

func begin_step(action: Dictionary) -> int:
	step_count += 1
	last_action = sanitize_variant(action)
	return step_count

func append_step_record(record: Dictionary) -> void:
	var file := FileAccess.open(steps_file_path, FileAccess.READ_WRITE)
	if file == null:
		push_error("AgentSession: failed to open step log at %s" % steps_file_path)
		return
	file.seek_end()
	file.store_line(JSON.stringify(sanitize_variant(record)))
	file.close()

func screenshot_path_for_step(step_index: int, stem: String = "capture") -> String:
	return "%s/%03d_%s.png" % [screenshots_dir, step_index, stem]

func finish(final_status: String, final_observation: Dictionary, result: Dictionary = {}) -> void:
	status = final_status
	ended_at_ms = Time.get_ticks_msec()
	last_result = sanitize_variant(result)
	current_chunk = _dict_to_vector2i(final_observation.get("current_chunk", current_chunk))
	player_position = _dict_to_vector3(final_observation.get("player_position", player_position))
	persist_summary(final_observation)

func persist_summary(current_observation: Dictionary = {}) -> void:
	var summary := {
		"session_id": session_id,
		"scenario_label": scenario_label,
		"status": status,
		"started_at_ms": started_at_ms,
		"ended_at_ms": ended_at_ms,
		"step_count": step_count,
		"world_seed": seed,
		"anchor_chunk": vector2i_to_dict(anchor_chunk),
		"current_chunk": vector2i_to_dict(current_chunk),
		"player_position": vector3_to_dict(player_position),
		"last_action": last_action,
		"last_result": last_result,
		"artifact_dir": artifact_dir,
		"artifact_dir_absolute": ProjectSettings.globalize_path(artifact_dir),
		"screenshots_dir": screenshots_dir,
		"screenshots_dir_absolute": ProjectSettings.globalize_path(screenshots_dir),
		"metadata": sanitize_variant(metadata),
	}
	if not current_observation.is_empty():
		summary["current_observation"] = sanitize_variant(current_observation)
	_write_text_file(session_file_path, JSON.stringify(sanitize_variant(summary), "\t"))

func summary() -> Dictionary:
	return sanitize_variant({
		"session_id": session_id,
		"scenario_label": scenario_label,
		"status": status,
		"started_at_ms": started_at_ms,
		"ended_at_ms": ended_at_ms,
		"step_count": step_count,
		"world_seed": seed,
		"anchor_chunk": anchor_chunk,
		"current_chunk": current_chunk,
		"player_position": player_position,
		"artifact_dir": artifact_dir,
		"screenshots_dir": screenshots_dir,
	})

static func sanitize_variant(value):
	if value is Dictionary:
		var sanitized := {}
		for key in value.keys():
			sanitized[str(key)] = sanitize_variant(value[key])
		return sanitized
	if value is Array:
		var array: Array = []
		for item in value:
			array.append(sanitize_variant(item))
		return array
	if value is Vector2i:
		return vector2i_to_dict(value)
	if value is Vector2:
		return {
			"x": value.x,
			"y": value.y,
		}
	if value is Vector3:
		return vector3_to_dict(value)
	if value is PackedInt32Array or value is PackedFloat32Array or value is PackedStringArray or value is PackedVector2Array or value is PackedVector3Array:
		var packed_array: Array = []
		for item in value:
			packed_array.append(sanitize_variant(item))
		return packed_array
	if value is PackedByteArray:
		var bytes: Array = []
		for item in value:
			bytes.append(int(item))
		return bytes
	return value

static func vector2i_to_dict(value: Vector2i) -> Dictionary:
	return {
		"x": value.x,
		"y": value.y,
	}

static func vector3_to_dict(value: Vector3) -> Dictionary:
	return {
		"x": value.x,
		"y": value.y,
		"z": value.z,
	}

func _ensure_artifact_dirs() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(screenshots_dir))

func _write_text_file(path: String, contents: String) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		push_error("AgentSession: failed to write %s" % path)
		return
	file.store_string(contents)
	file.close()

func _dict_to_vector2i(value) -> Vector2i:
	if value is Vector2i:
		return value
	if value is Dictionary:
		return Vector2i(int(value.get("x", 0)), int(value.get("y", 0)))
	if value is Vector2:
		return Vector2i(int(value.x), int(value.y))
	return Vector2i.ZERO

func _dict_to_vector3(value) -> Vector3:
	if value is Vector3:
		return value
	if value is Dictionary:
		return Vector3(
			float(value.get("x", 0.0)),
			float(value.get("y", 0.0)),
			float(value.get("z", 0.0))
		)
	return Vector3.ZERO
