extends Node

const AgentSessionScript = preload("res://scripts/autoload/agent_session.gd")
const AgentObservationBuilderScript = preload("res://scripts/autoload/agent_observation_builder.gd")
const AgentActionValidatorScript = preload("res://scripts/autoload/agent_action_validator.gd")
const ENABLE_ARG := "--agent-runtime"
const QUICK_LAUNCH_ARG := "--agent-runtime-quick-launch"
const SMOKE_ARG := "--agent-runtime-smoke-test"
const FLY_SWIM_SMOKE_ARG := "--fly-swim-smoke-test"
const ENABLE_ENV := "MG_AGENT_RUNTIME"
const BRIDGE_ROOT_ENV := "MG_AGENT_RUNTIME_BRIDGE_ROOT"
const MAX_MOVE_BLOCK_DISTANCE := GenerationManager.BLOCKS_PER_CHUNK * 2
const BRIDGE_SCHEMA_VERSION := 1
const BRIDGE_ROOT_DIR := "user://agent_runtime_bridge"
const BRIDGE_POLL_INTERVAL_SECONDS := 0.1

var _enabled := false
var _world = null
var _player: CharacterBody3D = null
var _head: Node3D = null
var _camera: Camera3D = null
var _chunk_streamer = null
var _session = null
var _observation_builder := AgentObservationBuilderScript.new()
var _action_validator := AgentActionValidatorScript.new()
var _current_action: Dictionary = {}
var _current_action_observation_before: Dictionary = {}
var _current_action_accepted_result: Dictionary = {}
var _completed_action_result: Dictionary = {}
var _completed_action_observation_before: Dictionary = {}
var _completed_action_observation_after: Dictionary = {}
var _smoke_test_started := false
var _fly_swim_smoke_started := false
var _bridge_runtime_id := ""
var _bridge_poll_remaining := 0.0
var _bridge_request_in_progress := false
var _bridge_pending_request: Dictionary = {}

func _ready() -> void:
	_enabled = _resolve_enabled()
	_bridge_runtime_id = "runtime_%d_%d" % [int(Time.get_unix_time_from_system()), Time.get_ticks_usec()]
	if _enabled:
		print("AgentRuntime: developer runtime enabled")
		_ensure_bridge_dirs()
		_write_bridge_state()
	if _wants_smoke_test():
		print("AgentRuntime: smoke test requested")

func _process(delta: float) -> void:
	if not _enabled:
		return
	if not _current_action.is_empty():
		var update := _update_current_action()
		if not update.is_empty():
			var action := _current_action
			_current_action = {}
			var after_observation := get_observation({
				"debug": true,
				"probe_points": _probe_points_for_action(action),
			})
			_finalize_step(action, update, after_observation)
			_store_completed_action(action, update, after_observation)
	_bridge_poll_remaining -= delta
	if _bridge_poll_remaining > 0.0:
		return
	_bridge_poll_remaining = BRIDGE_POLL_INTERVAL_SECONDS
	_write_bridge_state()
	if _bridge_request_in_progress:
		_update_bridge_pending_request()
	else:
		_poll_bridge_requests()

func is_enabled() -> bool:
	return _enabled

func has_runtime() -> bool:
	return _world != null and _player != null and _chunk_streamer != null

func has_active_session() -> bool:
	return _session != null and _session.status == "active"

func current_session_summary() -> Dictionary:
	return _session.summary() if _session != null else {}

func wants_smoke_test() -> bool:
	return _wants_smoke_test()

func register_world_runtime(world, player: CharacterBody3D, head: Node3D, camera: Camera3D, chunk_streamer) -> void:
	if not _enabled:
		return
	_world = world
	_player = player
	_head = head
	_camera = camera
	_chunk_streamer = chunk_streamer
	print("AgentRuntime: registered world runtime")
	_write_bridge_state()
	if _wants_smoke_test() and not _smoke_test_started:
		print("AgentRuntime: scheduling smoke test")
		_smoke_test_started = true
		call_deferred("_run_smoke_test")
	if _wants_fly_swim_smoke_test() and not _fly_swim_smoke_started:
		print("AgentRuntime: scheduling fly-swim smoke test")
		_fly_swim_smoke_started = true
		call_deferred("_run_fly_swim_smoke_test")

func unregister_world_runtime(world) -> void:
	if world == null or _world != world:
		return
	if not _current_action.is_empty():
		interrupt_current_action("World runtime exited.")
	_world = null
	_player = null
	_head = null
	_camera = null
	_chunk_streamer = null
	print("AgentRuntime: cleared world runtime")
	_write_bridge_state()

func start_session(goal_label: String = "playtest", metadata: Dictionary = {}) -> Dictionary:
	if not _enabled:
		return {
			"result": _result("rejected", "start_session", {}, "runtime_disabled", "Agent runtime is disabled.", "Run Godot with --agent-runtime in a debug/development context."),
			"observation": get_observation({"debug": true}),
		}
	if has_active_session():
		return {
			"result": _result("rejected", "start_session", {}, "session_already_active", "An agent session is already active.", "End the current session before starting another one."),
			"observation": get_observation({"debug": true}),
		}
	if not has_runtime():
		return {
			"result": _result("rejected", "start_session", {}, "no_world_runtime", "World runtime is not available yet.", "Load the world scene before starting an agent session."),
			"observation": get_observation({"debug": true}),
		}
	_session = AgentSessionScript.new()
	_clear_completed_action_state()
	var bootstrap_observation := get_observation({"debug": true})
	_session.configure(goal_label, bootstrap_observation, _session_metadata(metadata))
	var initial_observation := get_observation({"debug": true})
	_session.persist_summary(initial_observation)
	_append_bridge_event("session_started", {
		"session": _session.summary(),
	})
	_write_bridge_state()
	return {
		"result": _result("completed", "start_session", {
			"session": _session.summary(),
		}),
		"observation": initial_observation,
	}

func end_session(reason: String = "requested") -> Dictionary:
	if _session == null:
		return {
			"result": _result("rejected", "end_session", {}, "no_active_session", "No agent session is active.", "Start a session before ending it."),
			"observation": get_observation({"debug": true}),
		}
	if not _current_action.is_empty():
		var interrupted_action := _current_action
		_clear_scripted_motion()
		_current_action = {}
		var interrupted_result := _result(
			"interrupted",
			String(interrupted_action.get("name", "")),
			{},
			"action_interrupted",
			"Agent action was interrupted because the session ended.",
			reason
		)
		var interrupted_observation := get_observation({
			"debug": true,
			"probe_points": _probe_points_for_action(interrupted_action),
		})
		_finalize_step(interrupted_action, interrupted_result, interrupted_observation)
	var final_observation := get_observation({"debug": true})
	_session.finish("ended", final_observation, {
		"status": "completed",
		"reason": reason,
	})
	var result := _result("completed", "end_session", {
		"session": _session.summary(),
		"reason": reason,
	})
	_append_bridge_event("session_ended", {
		"session": _session.summary(),
		"reason": reason,
	})
	_session = null
	_write_bridge_state()
	return {
		"result": result,
		"observation": final_observation,
	}

func get_observation(options: Dictionary = {}) -> Dictionary:
	return _observation_builder.build(_world, _player, _head, _camera, _chunk_streamer, _session, options)

func submit_action(action_name: String, params: Dictionary = {}) -> Dictionary:
	if not _enabled:
		return _result("rejected", action_name, {}, "runtime_disabled", "Agent runtime is disabled.", "Run Godot with --agent-runtime.")
	if not has_active_session():
		return _result("rejected", action_name, {}, "no_active_session", "Start an agent session before submitting actions.", "Call `start_session()` first.")
	if not has_runtime():
		return _result("rejected", action_name, {}, "no_world_runtime", "World runtime is not available.", "Wait for the world scene to finish loading.")
	if not _current_action.is_empty():
		return _result("rejected", action_name, {}, "action_already_in_progress", "Another agent action is still running.", "Await or interrupt the current action before submitting a new one.")

	var validation := _action_validator.validate_action(action_name, params)
	if not bool(validation.get("ok", false)):
		return _result(
			"rejected",
			action_name,
			{},
			String(validation.get("error_code", "invalid_action")),
			String(validation.get("reason", "Invalid action.")),
			String(validation.get("hint", ""))
		)

	var action: Dictionary = validation["action"]
	_clear_completed_action_state()
	var step_index: int = int(_session.begin_step({
		"name": action_name,
		"params": action.get("params", {}),
	}))
	action["step_index"] = step_index
	action["started_at_ms"] = Time.get_ticks_msec()
	action["observation_before"] = get_observation({
		"debug": true,
		"probe_points": _probe_points_for_action(action),
	})
	_current_action_observation_before = action["observation_before"]
	var begin_result := _begin_action(action)
	if bool(begin_result.get("completed", false)):
		var final_result: Dictionary = begin_result["result"]
		var after_observation := get_observation({
			"debug": true,
			"probe_points": _probe_points_for_action(action),
		})
		_finalize_step(action, final_result, after_observation)
		return final_result

	_current_action = action
	_current_action_accepted_result = _result("accepted", action_name, {
		"step_index": step_index,
	})
	_write_bridge_state()
	return _current_action_accepted_result

func await_current_action() -> Dictionary:
	if _current_action.is_empty() and _completed_action_result.is_empty():
		return _result("rejected", "await_current_action", {}, "no_action_in_progress", "No agent action is running.", "Submit an action first.")
	while _completed_action_result.is_empty():
		await get_tree().process_frame
	return _consume_completed_action_result()

func run_step(action_name: String, params: Dictionary = {}) -> Dictionary:
	var submit_result := submit_action(action_name, params)
	if String(submit_result.get("status", "")) != "accepted":
		return {
			"result": submit_result,
			"observation_before": _current_action_observation_before if not _current_action_observation_before.is_empty() else get_observation({"debug": true}),
			"observation": get_observation({"debug": true}),
		}
	var final_result := await await_current_action()
	return {
		"result": final_result,
		"observation_before": _completed_action_observation_before if not _completed_action_observation_before.is_empty() else _current_action_observation_before,
		"observation": _completed_action_observation_after if not _completed_action_observation_after.is_empty() else get_observation({"debug": true}),
	}

func interrupt_current_action(reason: String = "Interrupted by caller.") -> Dictionary:
	if _current_action.is_empty():
		return _result("rejected", "interrupt_current_action", {}, "no_action_in_progress", "No agent action is running.", "")
	var action := _current_action
	_clear_scripted_motion()
	_current_action = {}
	var result := _result("interrupted", String(action.get("name", "")), {}, "action_interrupted", "Agent action was interrupted.", reason)
	var after_observation := get_observation({
		"debug": true,
		"probe_points": _probe_points_for_action(action),
	})
	_finalize_step(action, result, after_observation)
	_store_completed_action(action, result, after_observation)
	_write_bridge_state()
	return _result("completed", "interrupt_current_action", {
		"reason": reason,
	})

func _begin_action(action: Dictionary) -> Dictionary:
	var action_name := String(action.get("name", ""))
	match action_name:
		"look_at":
			return _complete_immediately(_handle_look_at(action))
		"teleport_to_block":
			return _complete_immediately(_handle_teleport(action))
		"sample_height":
			return _complete_immediately(_handle_sample_height(action))
		"find_nearest_land":
			return _complete_immediately(_handle_find_nearest_land(action))
		"capture_screenshot":
			return _complete_immediately(_handle_capture_screenshot(action))
		"get_chunk_state":
			return _complete_immediately(_handle_get_chunk_state(action))
		"wait_for_chunk_loaded":
			return _begin_wait_for_chunk_loaded(action)
		"wait_for_ring_ready":
			return _begin_wait_for_ring_ready(action)
		"wait_for_player_settled":
			return {"completed": false}
		"end_session":
			return _complete_immediately(_handle_end_session_action(action))
		"stop":
			if _player and _player.has_method("clear_scripted_motion"):
				_player.call("clear_scripted_motion")
			return {"completed": false}
		"wait_seconds":
			return {"completed": false}
		"move_in_direction":
			_player.call("set_scripted_motion", action["params"]["direction"], float(action["params"]["speed"]))
			return {"completed": false}
		"move_to_block":
			return _begin_move_to_block(action)
		"toggle_fly":
			return _complete_immediately(_handle_toggle_fly())
		"set_fly_vertical":
			return _complete_immediately(_handle_set_fly_vertical(action))
		"get_move_state":
			return _complete_immediately(_handle_get_move_state())
		_:
			return _complete_immediately(_result("rejected", action_name, {}, "unsupported_action", "Unsupported agent action: %s" % action_name, ""))

func _update_current_action() -> Dictionary:
	if _current_action.is_empty():
		return {}
	if not has_runtime():
		return _result("interrupted", String(_current_action.get("name", "")), {}, "no_world_runtime", "World runtime is no longer available.", "World runtime exited while the action was still running.")
	var action_name := String(_current_action.get("name", ""))
	var started_at_ms := int(_current_action.get("started_at_ms", Time.get_ticks_msec()))
	var elapsed_seconds := maxf(0.0, float(Time.get_ticks_msec() - started_at_ms) / 1000.0)
	var params: Dictionary = _current_action.get("params", {})
	var timeout_seconds := float(params.get("timeout_seconds", 0.0))
	if timeout_seconds > 0.0 and elapsed_seconds >= timeout_seconds:
		_clear_scripted_motion()
		return _result("timed_out", action_name, {}, "action_timed_out", "Agent action exceeded its timeout.", "")

	match action_name:
		"wait_seconds":
			if elapsed_seconds >= float(params.get("duration_seconds", 0.0)):
				return _result("completed", action_name, {
					"duration_seconds": float(params.get("duration_seconds", 0.0)),
				})
		"stop":
			if _horizontal_speed() <= float(params.get("stop_tolerance", 0.25)):
				return _result("completed", action_name, {
					"horizontal_speed": _horizontal_speed(),
				})
		"move_in_direction":
			if elapsed_seconds >= float(params.get("duration_seconds", 0.0)):
				_clear_scripted_motion()
				if _horizontal_speed() <= float(params.get("stop_tolerance", 0.25)):
					return _result("completed", action_name, {
						"duration_seconds": float(params.get("duration_seconds", 0.0)),
						"horizontal_speed": _horizontal_speed(),
					})
		"move_to_block":
			return _update_move_to_block_action(_current_action)
		"wait_for_chunk_loaded":
			return _update_wait_for_chunk_loaded_action(_current_action)
		"wait_for_ring_ready":
			return _update_wait_for_ring_ready_action(_current_action)
		"wait_for_player_settled":
			return _update_wait_for_player_settled_action()
	return {}

func _handle_toggle_fly() -> Dictionary:
	if _player == null or not _player.has_method("toggle_fly"):
		return _result("rejected", "toggle_fly", {}, "no_player", "Player not available or missing toggle_fly method.", "")
	_player.call("toggle_fly")
	return _result("completed", "toggle_fly", {"move_state": _read_move_state_name()})

func _handle_set_fly_vertical(action: Dictionary) -> Dictionary:
	if _player == null or not _player.has_method("set_scripted_fly_vertical"):
		return _result("rejected", "set_fly_vertical", {}, "no_player", "Player not available or missing set_scripted_fly_vertical method.", "")
	var move_state := _read_move_state_name()
	if move_state != "FLYING":
		return _result("rejected", "set_fly_vertical", {}, "not_flying", "set_fly_vertical requires FLYING state, currently %s." % move_state, "Use toggle_fly first.")
	var vertical := float(action.get("params", {}).get("vertical", 0.0))
	_player.call("set_scripted_fly_vertical", vertical)
	return _result("completed", "set_fly_vertical", {"vertical": vertical, "move_state": move_state})

func _handle_get_move_state() -> Dictionary:
	if _player == null:
		return _result("rejected", "get_move_state", {}, "no_player", "Player not available.", "")
	return _result("completed", "get_move_state", {
		"move_state": _read_move_state_name(),
		"player_position": AgentSessionScript.vector3_to_dict(_player.position),
		"player_velocity": AgentSessionScript.vector3_to_dict(_player.velocity),
	})

func _read_move_state_name() -> String:
	if _player == null:
		return "unknown"
	var state_val = _player.get("_move_state")
	if state_val == null:
		return "unknown"
	match int(state_val):
		0: return "WALKING"
		1: return "FLYING"
		2: return "SWIMMING"
	return "unknown"

func _begin_move_to_block(action: Dictionary) -> Dictionary:
	var target_block: Vector2i = action["params"]["scene_block"]
	if _distance_to_scene_block(target_block) > float(MAX_MOVE_BLOCK_DISTANCE):
		return _complete_immediately(
			_result(
				"rejected",
				"move_to_block",
				{},
				"target_block_out_of_supported_range",
				"Target block is outside the supported movement range for phase 1.",
				"Keep `move_to_block` targets within roughly two chunks of the player."
			)
		)
	var target_chunk := GenerationManager.scene_block_to_chunk_coord(
		GameState.anchor_chunk,
		target_block.x,
		target_block.y
	)
	action["target_chunk"] = target_chunk
	_current_action = action
	_update_move_direction_to_target(action)
	return {"completed": false}

func _update_move_to_block_action(action: Dictionary) -> Dictionary:
	var target_block: Vector2i = action["params"]["scene_block"]
	var arrival_radius := float(action["params"].get("arrival_radius", 1.25))
	if _distance_to_scene_block(target_block) <= arrival_radius:
		_clear_scripted_motion()
		if _horizontal_speed() <= float(action["params"].get("stop_tolerance", 0.25)):
			return _result("completed", "move_to_block", {
				"scene_block": AgentSessionScript.sanitize_variant(target_block),
				"target_chunk": AgentSessionScript.sanitize_variant(action.get("target_chunk", Vector2i.ZERO)),
			})
		return {}
	_update_move_direction_to_target(action)
	return {}

func _update_move_direction_to_target(action: Dictionary) -> void:
	var target_block: Vector2i = action["params"]["scene_block"]
	var target_position := Vector3(float(target_block.x) + 0.5, _player.position.y, float(target_block.y) + 0.5)
	var direction := target_position - _player.position
	direction.y = 0.0
	if direction.length_squared() <= 0.0001:
		_clear_scripted_motion()
		return
	_player.call("set_scripted_motion", direction.normalized(), float(action["params"]["speed"]))

func _begin_wait_for_chunk_loaded(action: Dictionary) -> Dictionary:
	action["target_chunk"] = _resolve_action_chunk(action["params"])
	return {"completed": false}

func _update_wait_for_chunk_loaded_action(action: Dictionary) -> Dictionary:
	var target_chunk: Vector2i = action.get("target_chunk", GameState.current_chunk)
	var required_lod := String(action["params"].get("required_lod", ""))
	var chunk_state: Dictionary = _world.get_chunk_state(target_chunk)
	if not bool(chunk_state.get("loaded", false)):
		return {}
	if not required_lod.is_empty() and String(chunk_state.get("lod", "")) != required_lod:
		return {}
	return _result("completed", "wait_for_chunk_loaded", {
		"chunk_state": AgentSessionScript.sanitize_variant(chunk_state),
	})

func _begin_wait_for_ring_ready(action: Dictionary) -> Dictionary:
	action["target_chunk"] = _resolve_action_chunk(action["params"])
	return {"completed": false}

func _update_wait_for_ring_ready_action(action: Dictionary) -> Dictionary:
	var target_chunk: Vector2i = action.get("target_chunk", GameState.current_chunk)
	var radius := int(action["params"].get("radius", 1))
	if _chunk_streamer == null or not _chunk_streamer.is_ring_ready(target_chunk, radius):
		return {}
	return _result("completed", "wait_for_ring_ready", {
		"center_chunk": AgentSessionScript.sanitize_variant(target_chunk),
		"radius": radius,
		"current_loaded_chunk_counts_by_lod": AgentSessionScript.sanitize_variant(_chunk_streamer.active_counts_by_lod()),
	})

func _update_wait_for_player_settled_action() -> Dictionary:
	var params: Dictionary = _current_action.get("params", {})
	if not _player_is_settled(params):
		_current_action.erase("stable_started_at_ms")
		return {}
	var stable_started_at_ms := int(_current_action.get("stable_started_at_ms", 0))
	if stable_started_at_ms <= 0:
		_current_action["stable_started_at_ms"] = Time.get_ticks_msec()
		return {}
	var stable_duration_seconds := float(params.get("stable_duration_seconds", 0.0))
	if stable_duration_seconds > 0.0:
		var stable_elapsed := float(Time.get_ticks_msec() - stable_started_at_ms) / 1000.0
		if stable_elapsed < stable_duration_seconds:
			return {}
	return _result("completed", "wait_for_player_settled", {
		"horizontal_speed": _horizontal_speed(),
		"vertical_speed": absf(_player.velocity.y),
		"on_floor": _player.is_on_floor(),
	})

func _handle_look_at(action: Dictionary) -> Dictionary:
	var params: Dictionary = action["params"]
	var target_point = params.get("target_point", null)
	if target_point == null:
		var scene_block: Vector2i = params["scene_block"]
		var surface_y: float = float(_world.sample_surface_height(scene_block.x, scene_block.y))
		target_point = Vector3(
			float(scene_block.x) + 0.5,
			surface_y + float(params.get("height_offset", 0.0)),
			float(scene_block.y) + 0.5
		)
	if not _player.has_method("set_scripted_look_at"):
		return _result("rejected", "look_at", {}, "missing_look_helper", "Player controller does not expose scripted look helpers.", "")
	_player.call("set_scripted_look_at", target_point)
	return _result("completed", "look_at", {
		"target_point": AgentSessionScript.sanitize_variant(target_point),
		"tolerance_degrees": float(params.get("tolerance_degrees", 1.0)),
	})

func _handle_teleport(action: Dictionary) -> Dictionary:
	var params: Dictionary = action["params"]
	var scene_block: Vector2i = params["scene_block"]
	var target_chunk := GenerationManager.scene_block_to_chunk_coord(
		GameState.anchor_chunk,
		scene_block.x,
		scene_block.y
	)
	if not _world.has_method("is_chunk_loaded") or not bool(_world.call("is_chunk_loaded", target_chunk)):
		return _result(
			"rejected",
			"teleport_to_block",
			{},
			"target_chunk_not_loaded_yet",
			"Teleport target chunk is not currently loaded.",
			"Use `get_chunk_state` first or move closer before teleporting."
		)
	_clear_scripted_motion()
	var height := float(_world.sample_surface_height(scene_block.x, scene_block.y))
	_player.position = Vector3(
		float(scene_block.x) + 0.5,
		height + float(params.get("height_offset", 3.0)),
		float(scene_block.y) + 0.5
	)
	_player.velocity = Vector3.ZERO
	return _result("completed", "teleport_to_block", {
		"scene_block": AgentSessionScript.sanitize_variant(scene_block),
		"target_chunk": AgentSessionScript.sanitize_variant(target_chunk),
		"surface_height": height,
	})

func _handle_sample_height(action: Dictionary) -> Dictionary:
	var scene_block: Vector2i = action["params"]["scene_block"]
	return _result("completed", "sample_height", {
		"scene_block": AgentSessionScript.sanitize_variant(scene_block),
		"height": float(_world.sample_surface_height(scene_block.x, scene_block.y)),
	})

func _handle_find_nearest_land(action: Dictionary) -> Dictionary:
	var scene_block: Vector2i = action["params"]["scene_block"]
	var nearest_land: Vector2 = _world.nearest_land_block(scene_block.x, scene_block.y)
	return _result("completed", "find_nearest_land", {
		"requested_scene_block": {
			"x": scene_block.x,
			"z": scene_block.y,
		},
		"nearest_land_scene_block": {
			"x": int(round(nearest_land.x)),
			"z": int(round(nearest_land.y)),
		},
		"height": float(_world.sample_surface_height(int(round(nearest_land.x)), int(round(nearest_land.y)))),
	})

func _handle_capture_screenshot(action: Dictionary) -> Dictionary:
	var params: Dictionary = action["params"]
	var path := String(params.get("path", ""))
	if path.is_empty():
		var file_name := String(params.get("file_name", "capture")).strip_edges()
		if file_name.is_empty():
			file_name = "capture"
		path = _session.screenshot_path_for_step(int(action["step_index"]), file_name)
	var capture_result: Dictionary = Flythrough.capture_screenshot_to_path(path)
	if not bool(capture_result.get("ok", false)):
		return _result(
			"rejected",
			"capture_screenshot",
			{
				"path": String(capture_result.get("path", path)),
				"absolute_path": String(capture_result.get("absolute_path", ProjectSettings.globalize_path(path))),
			},
			String(capture_result.get("error_code", "screenshot_capture_failed")),
			String(capture_result.get("error", "Failed to capture screenshot to the requested path.")),
			"Use a non-headless display driver if you need image output."
		)
	return _result("completed", "capture_screenshot", capture_result)

func _handle_get_chunk_state(action: Dictionary) -> Dictionary:
	var params: Dictionary = action.get("params", {})
	var chunk_coord: Vector2i = _resolve_action_chunk(params)
	return _result("completed", "get_chunk_state", {
		"chunk_state": AgentSessionScript.sanitize_variant(_world.get_chunk_state(chunk_coord)),
	})

func _handle_end_session_action(action: Dictionary) -> Dictionary:
	return _result("completed", "end_session", {
		"reason": "end_session action",
	})

func _complete_immediately(result: Dictionary) -> Dictionary:
	return {
		"completed": true,
		"result": result,
	}

func _finalize_step(action: Dictionary, result: Dictionary, after_observation: Dictionary) -> void:
	if _session == null:
		return
	_current_action_observation_before = action.get("observation_before", {})
	_session.current_chunk = GameState.current_chunk
	_session.player_position = _player.position if _player != null else _session.player_position
	if String(action.get("name", "")) == "end_session":
		_session.finish("ended", after_observation, result)
		var finalized_result := result.duplicate(true)
		var finalized_data: Dictionary = finalized_result.get("data", {})
		finalized_data["session"] = _session.summary()
		finalized_result["data"] = finalized_data
		result.clear()
		result.merge(finalized_result, true)
	_session.last_result = AgentSessionScript.sanitize_variant(result)
	_session.append_step_record({
		"schema_version": 1,
		"session_id": _session.session_id,
		"step_index": int(action.get("step_index", _session.step_count)),
		"timestamp_ms": Time.get_ticks_msec(),
		"action": {
			"name": action.get("name", ""),
			"params": AgentSessionScript.sanitize_variant(action.get("params", {})),
		},
		"result": result,
		"observation_before": action.get("observation_before", {}),
		"observation_after": after_observation,
	})
	_append_bridge_event("step_completed", {
		"action": {
			"name": action.get("name", ""),
			"params": AgentSessionScript.sanitize_variant(action.get("params", {})),
		},
		"result": result,
	})
	if String(action.get("name", "")) == "end_session":
		_session = null
	else:
		_session.persist_summary(after_observation)
	_write_bridge_state()

func _probe_points_for_action(action: Dictionary) -> Array:
	var params: Dictionary = action.get("params", {})
	var points: Array = []
	if params.get("scene_block", null) != null:
		points.append(params["scene_block"])
	return points

func _session_metadata(extra_metadata: Dictionary) -> Dictionary:
	var metadata := extra_metadata.duplicate(true)
	metadata["launch_mode"] = GameState.runtime_launch_mode_name()
	metadata["launch_world_origin"] = AgentSessionScript.sanitize_variant(GameState.runtime_launch_world_origin)
	metadata["launch_chunk"] = AgentSessionScript.sanitize_variant(GameState.runtime_launch_chunk)
	metadata["agent_runtime_enabled"] = _enabled
	metadata["agent_runtime_gate"] = {
		"arg": ENABLE_ARG,
		"env": ENABLE_ENV,
		"quick_launch_arg": QUICK_LAUNCH_ARG,
	}
	metadata["cmdline_args"] = OS.get_cmdline_args()
	metadata["cmdline_user_args"] = OS.get_cmdline_user_args()
	metadata["bridge_root"] = _bridge_root_dir()
	metadata["bridge_root_absolute"] = ProjectSettings.globalize_path(_bridge_root_dir())
	return metadata

func _bridge_root_dir() -> String:
	var override_path := OS.get_environment(BRIDGE_ROOT_ENV).strip_edges()
	return override_path if not override_path.is_empty() else BRIDGE_ROOT_DIR

func _bridge_requests_dir() -> String:
	return "%s/requests" % _bridge_root_dir()

func _bridge_responses_dir() -> String:
	return "%s/responses" % _bridge_root_dir()

func _bridge_state_path() -> String:
	return "%s/state.json" % _bridge_root_dir()

func _bridge_events_path() -> String:
	return "%s/events.jsonl" % _bridge_root_dir()

func _resolve_action_chunk(params: Dictionary) -> Vector2i:
	var chunk_coord = params.get("chunk_coord", null)
	if chunk_coord != null:
		return chunk_coord
	var scene_block = params.get("scene_block", null)
	if scene_block != null:
		return GenerationManager.scene_block_to_chunk_coord(
			GameState.anchor_chunk,
			scene_block.x,
			scene_block.y
		)
	return GameState.current_chunk

func _player_is_settled(params: Dictionary) -> bool:
	if _player == null:
		return false
	if bool(params.get("require_on_floor", true)) and not _player.is_on_floor():
		return false
	if _horizontal_speed() > float(params.get("horizontal_speed_tolerance", 0.25)):
		return false
	if absf(_player.velocity.y) > float(params.get("vertical_speed_tolerance", 0.5)):
		return false
	return true

func _store_completed_action(action: Dictionary, result: Dictionary, after_observation: Dictionary) -> void:
	_completed_action_result = result.duplicate(true)
	_completed_action_observation_before = AgentSessionScript.sanitize_variant(action.get("observation_before", {}))
	_completed_action_observation_after = AgentSessionScript.sanitize_variant(after_observation)

func _consume_completed_action_result() -> Dictionary:
	var result := _completed_action_result.duplicate(true)
	_completed_action_result = {}
	return result

func _clear_completed_action_state() -> void:
	_completed_action_result = {}
	_completed_action_observation_before = {}
	_completed_action_observation_after = {}

func _distance_to_scene_block(scene_block: Vector2i) -> float:
	var dx := _player.position.x - (float(scene_block.x) + 0.5)
	var dz := _player.position.z - (float(scene_block.y) + 0.5)
	return sqrt(dx * dx + dz * dz)

func _horizontal_speed() -> float:
	if _player == null:
		return 0.0
	return Vector2(_player.velocity.x, _player.velocity.z).length()

func _clear_scripted_motion() -> void:
	if _player != null and _player.has_method("clear_scripted_motion"):
		_player.call("clear_scripted_motion")

func _resolve_enabled() -> bool:
	if OS.has_method("is_debug_build"):
		if not OS.is_debug_build():
			return false
	elif not OS.has_feature("editor"):
		return false
	var cmdline_args := _all_cmdline_args()
	if ENABLE_ARG in cmdline_args or SMOKE_ARG in cmdline_args or FLY_SWIM_SMOKE_ARG in cmdline_args:
		return true
	var env_value := OS.get_environment(ENABLE_ENV).to_lower()
	return env_value == "1" or env_value == "true" or env_value == "yes"

func _wants_smoke_test() -> bool:
	return SMOKE_ARG in _all_cmdline_args()

func _wants_fly_swim_smoke_test() -> bool:
	return FLY_SWIM_SMOKE_ARG in _all_cmdline_args()

func _all_cmdline_args() -> Array:
	var args: Array = []
	args.append_array(OS.get_cmdline_args())
	args.append_array(OS.get_cmdline_user_args())
	return args

func _ensure_bridge_dirs() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(_bridge_requests_dir()))
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(_bridge_responses_dir()))

func _bridge_state_payload() -> Dictionary:
	var current_runtime_presentation: Dictionary = {}
	if has_runtime() and _world != null and _world.has_method("get_current_runtime_presentation"):
		current_runtime_presentation = AgentSessionScript.sanitize_variant(_world.get_current_runtime_presentation())
	return {
		"schema_version": BRIDGE_SCHEMA_VERSION,
		"transport": "file_runtime_bridge",
		"runtime_id": _bridge_runtime_id,
		"timestamp_ms": Time.get_ticks_msec(),
		"enabled": _enabled,
		"display_driver": DisplayServer.get_name(),
		"runtime_available": has_runtime(),
		"active_session": has_active_session(),
		"session": current_session_summary(),
		"current_action": {
			"name": String(_current_action.get("name", "")),
			"in_progress": not _current_action.is_empty(),
			"step_index": int(_current_action.get("step_index", 0)),
		},
		"bridge_request_in_progress": _bridge_request_in_progress,
		"bridge_pending_request": {
			"request_id": String(_bridge_pending_request.get("request_id", "")),
			"command": String(_bridge_pending_request.get("command", "")),
			"mode": String(_bridge_pending_request.get("mode", "")),
		},
		"paths": {
			"root": _bridge_root_dir(),
			"root_absolute": ProjectSettings.globalize_path(_bridge_root_dir()),
			"requests": _bridge_requests_dir(),
			"requests_absolute": ProjectSettings.globalize_path(_bridge_requests_dir()),
			"responses": _bridge_responses_dir(),
			"responses_absolute": ProjectSettings.globalize_path(_bridge_responses_dir()),
			"state": _bridge_state_path(),
			"state_absolute": ProjectSettings.globalize_path(_bridge_state_path()),
			"events": _bridge_events_path(),
			"events_absolute": ProjectSettings.globalize_path(_bridge_events_path()),
		},
		"runtime_constants": {
			"blocks_per_chunk": GenerationManager.BLOCKS_PER_CHUNK,
			"world_units_per_chunk": GenerationManager.WORLD_UNITS_PER_CHUNK,
		},
		"current_chunk_runtime_presentation": {
			"planet_zone": current_runtime_presentation.get("planet_zone", {}),
			"atmosphere_class": current_runtime_presentation.get("atmosphere_class", {}),
			"water_state": current_runtime_presentation.get("water_state", {}),
			"landform_class": current_runtime_presentation.get("landform_class", {}),
			"surface_palette_class": current_runtime_presentation.get("surface_palette_class", {}),
			"interestingness_score": current_runtime_presentation.get("interestingness_score", 0.0),
			"reduced_grids": current_runtime_presentation.get("reduced_grids", {}),
		},
		"supported_commands": [
			"ping",
			"get_state",
			"get_observation",
			"current_session_summary",
			"start_session",
			"end_session",
			"submit_action",
			"await_current_action",
			"run_step",
			"interrupt_current_action",
		],
		"supported_actions": [
			"teleport_to_block",
			"look_at",
			"move_in_direction",
			"move_to_block",
			"stop",
			"sample_height",
			"find_nearest_land",
			"wait_seconds",
			"capture_screenshot",
			"get_chunk_state",
			"wait_for_chunk_loaded",
			"wait_for_ring_ready",
			"wait_for_player_settled",
			"end_session",
		],
	}

func _write_bridge_state() -> void:
	if not _enabled:
		return
	_ensure_bridge_dirs()
	_write_json_file(_bridge_state_path(), _bridge_state_payload())

func _append_bridge_event(event_name: String, payload: Dictionary) -> void:
	if not _enabled:
		return
	_ensure_bridge_dirs()
	var file := FileAccess.open(_bridge_events_path(), FileAccess.READ_WRITE)
	if file == null:
		file = FileAccess.open(_bridge_events_path(), FileAccess.WRITE)
	if file == null:
		return
	file.seek_end()
	file.store_line(JSON.stringify(AgentSessionScript.sanitize_variant({
		"schema_version": BRIDGE_SCHEMA_VERSION,
		"runtime_id": _bridge_runtime_id,
		"event": event_name,
		"timestamp_ms": Time.get_ticks_msec(),
		"payload": payload,
	})))
	file.close()

func _poll_bridge_requests() -> void:
	var dir := DirAccess.open(_bridge_requests_dir())
	if dir == null:
		return
	var pending_files: Array[String] = []
	dir.list_dir_begin()
	var file_name := dir.get_next()
	while not file_name.is_empty():
		if not dir.current_is_dir() and file_name.ends_with(".json"):
			pending_files.append(file_name)
		file_name = dir.get_next()
	dir.list_dir_end()
	if pending_files.is_empty():
		return
	pending_files.sort()
	var request_path := "%s/%s" % [_bridge_requests_dir(), pending_files[0]]
	var request: Dictionary = _read_json_file(request_path)
	if request.is_empty():
		_write_bridge_response(request_path, pending_files[0].trim_suffix(".json"), "invalid_request", false, {}, "invalid_json", "Request file could not be parsed as JSON.")
		_delete_file(request_path)
		return
	_dispatch_bridge_request(request_path, request)

func _dispatch_bridge_request(request_path: String, request: Dictionary) -> void:
	var request_id := String(request.get("request_id", request_path.get_file().trim_suffix(".json"))).strip_edges()
	if request_id.is_empty():
		request_id = "request_%d" % Time.get_ticks_usec()
	var command := String(request.get("command", "")).strip_edges()
	var args: Dictionary = request.get("args", {})
	if command.is_empty():
		_write_bridge_response(request_path, request_id, "", false, {}, "missing_command", "Bridge request is missing a command.")
		_delete_file(request_path)
		return

	match command:
		"ping":
			_write_bridge_response(request_path, request_id, command, true, {
				"message": "pong",
				"state": _bridge_state_payload(),
			})
			_delete_file(request_path)
		"get_state":
			_write_bridge_response(request_path, request_id, command, true, {
				"state": _bridge_state_payload(),
			})
			_delete_file(request_path)
		"get_observation":
			_write_bridge_response(request_path, request_id, command, true, {
				"observation": get_observation(args.get("options", {})),
			})
			_delete_file(request_path)
		"current_session_summary":
			_write_bridge_response(request_path, request_id, command, true, {
				"session": current_session_summary(),
			})
			_delete_file(request_path)
		"start_session":
			_write_bridge_response(request_path, request_id, command, true, start_session(
				String(args.get("goal_label", "playtest")),
				args.get("metadata", {})
			))
			_delete_file(request_path)
		"end_session":
			_write_bridge_response(request_path, request_id, command, true, end_session(
				String(args.get("reason", "bridge_request"))
			))
			_delete_file(request_path)
		"submit_action":
			var action_name := String(args.get("action", args.get("action_name", "")))
			var submit_result := submit_action(action_name, args.get("params", {}))
			_write_bridge_response(request_path, request_id, command, true, {
				"result": submit_result,
				"observation_before": _current_action_observation_before if not _current_action_observation_before.is_empty() else get_observation({"debug": true}),
				"observation": get_observation({"debug": true}),
			})
			_delete_file(request_path)
		"interrupt_current_action":
			_write_bridge_response(request_path, request_id, command, true, {
				"result": interrupt_current_action(String(args.get("reason", "Interrupted by bridge request."))),
				"observation": get_observation({"debug": true}),
			})
			_delete_file(request_path)
		"await_current_action":
			if _current_action.is_empty() and _completed_action_result.is_empty():
				_write_bridge_response(
					request_path,
					request_id,
					command,
					true,
					{
						"result": _result("rejected", "await_current_action", {}, "no_action_in_progress", "No agent action is running.", "Submit an action first."),
						"observation": get_observation({"debug": true}),
					}
				)
				_delete_file(request_path)
				return
			_bridge_request_in_progress = true
			_bridge_pending_request = {
				"request_path": request_path,
				"request_id": request_id,
				"command": command,
				"mode": "await_current_action",
			}
			_write_bridge_state()
		"run_step":
			var requested_action := String(args.get("action", args.get("action_name", "")))
			var run_submit_result := submit_action(requested_action, args.get("params", {}))
			if String(run_submit_result.get("status", "")) != "accepted":
				_write_bridge_response(request_path, request_id, command, true, {
					"result": run_submit_result,
					"observation_before": _current_action_observation_before if not _current_action_observation_before.is_empty() else get_observation({"debug": true}),
					"observation": get_observation({"debug": true}),
				})
				_delete_file(request_path)
				return
			_bridge_request_in_progress = true
			_bridge_pending_request = {
				"request_path": request_path,
				"request_id": request_id,
				"command": command,
				"mode": "run_step",
			}
			_write_bridge_state()
		_:
			_write_bridge_response(request_path, request_id, command, false, {}, "unsupported_command", "Unsupported bridge command: %s" % command)
			_delete_file(request_path)

func _update_bridge_pending_request() -> void:
	if _bridge_pending_request.is_empty():
		_bridge_request_in_progress = false
		return
	if _current_action.is_empty() and _completed_action_result.is_empty():
		return
	if not _current_action.is_empty():
		return

	var request_path := String(_bridge_pending_request.get("request_path", ""))
	var request_id := String(_bridge_pending_request.get("request_id", ""))
	var command := String(_bridge_pending_request.get("command", ""))
	var payload := {
		"result": _consume_completed_action_result(),
		"observation_before": _completed_action_observation_before,
		"observation": _completed_action_observation_after if not _completed_action_observation_after.is_empty() else get_observation({"debug": true}),
	}
	_write_bridge_response(request_path, request_id, command, true, payload)
	_delete_file(request_path)
	_bridge_pending_request = {}
	_bridge_request_in_progress = false
	_write_bridge_state()

func _write_bridge_response(
		request_path: String,
		request_id: String,
		command: String,
		ok: bool,
		payload: Dictionary = {},
		error_code: String = "",
		error_message: String = "") -> void:
	var response_path := "%s/%s.json" % [_bridge_responses_dir(), request_id]
	var response := {
		"schema_version": BRIDGE_SCHEMA_VERSION,
		"transport": "file_runtime_bridge",
		"runtime_id": _bridge_runtime_id,
		"request_id": request_id,
		"command": command,
		"timestamp_ms": Time.get_ticks_msec(),
		"ok": ok,
		"payload": AgentSessionScript.sanitize_variant(payload),
		"request_path": request_path,
	}
	if not error_code.is_empty():
		response["error_code"] = error_code
	if not error_message.is_empty():
		response["error"] = error_message
	_write_json_file(response_path, response)

func _read_json_file(path: String) -> Dictionary:
	if not FileAccess.file_exists(path):
		return {}
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return {}
	var contents := file.get_as_text()
	file.close()
	var parsed = JSON.parse_string(contents)
	return parsed if parsed is Dictionary else {}

func _write_json_file(path: String, data: Dictionary) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return
	file.store_string(JSON.stringify(AgentSessionScript.sanitize_variant(data), "\t"))
	file.close()

func _delete_file(path: String) -> void:
	if not FileAccess.file_exists(path):
		return
	DirAccess.remove_absolute(ProjectSettings.globalize_path(path))

func _run_smoke_test() -> void:
	print("AgentRuntime: starting smoke test")
	await get_tree().process_frame
	await get_tree().create_timer(0.5).timeout

	var smoke_result := await _execute_smoke_test()
	var ok := bool(smoke_result.get("ok", false))
	print("AgentRuntime smoke test: %s" % ["PASS" if ok else "FAIL"])
	print(JSON.stringify(AgentSessionScript.sanitize_variant(smoke_result), "\t"))
	get_tree().quit(0 if ok else 1)

func _execute_smoke_test() -> Dictionary:
	var started := start_session("agent_runtime_smoke_test", {
		"scenario": "agent_runtime_smoke_test",
	})
	var start_result: Dictionary = started.get("result", {})
	if String(start_result.get("status", "")) != "completed":
		return {
			"ok": false,
			"phase": "start_session",
			"result": started,
		}

	var initial_observation: Dictionary = started.get("observation", {})
	var initial_chunk := _dict_to_vector2i(initial_observation.get("current_chunk", GameState.current_chunk))
	var current_origin := GenerationManager.chunk_coord_to_scene_origin(initial_chunk, GameState.anchor_chunk)
	var seam_z := int(round(current_origin.z + float(GenerationManager.BLOCKS_PER_CHUNK) * 0.5))
	var teleport_probe := Vector2i(
		int(round(current_origin.x + float(GenerationManager.BLOCKS_PER_CHUNK) - 16.0)),
		seam_z
	)
	var target_probe := Vector2i(
		int(round(current_origin.x + float(GenerationManager.BLOCKS_PER_CHUNK) + 24.0)),
		seam_z
	)

	var teleport_land_step := await run_step("find_nearest_land", {
		"scene_block": teleport_probe,
	})
	if not _step_succeeded(teleport_land_step):
		return _with_session_cleanup(false, "find_teleport_land", teleport_land_step)
	var teleport_land := _step_scene_block(teleport_land_step, "nearest_land_scene_block")

	var teleport_step := await run_step("teleport_to_block", {
		"scene_block": teleport_land,
	})
	if not _step_succeeded(teleport_step):
		return _with_session_cleanup(false, "teleport_to_block", teleport_step)

	var settle_step := await run_step("wait_seconds", {
		"duration_seconds": 0.6,
		"timeout_seconds": 1.5,
	})
	if not _step_succeeded(settle_step):
		return _with_session_cleanup(false, "wait_after_teleport", settle_step)

	var land_step := await run_step("find_nearest_land", {
		"scene_block": target_probe,
	})
	if not _step_succeeded(land_step):
		return _with_session_cleanup(false, "find_move_land", land_step)
	var move_target := _step_scene_block(land_step, "nearest_land_scene_block")

	var move_step := await run_step("move_to_block", {
		"scene_block": move_target,
		"speed": 12.0,
		"arrival_radius": 1.5,
		"timeout_seconds": 6.0,
	})
	if not _step_succeeded(move_step):
		return _with_session_cleanup(false, "move_to_block", move_step)

	var moved_observation: Dictionary = move_step.get("observation", {})
	var moved_chunk := _dict_to_vector2i(moved_observation.get("current_chunk", GameState.current_chunk))
	if moved_chunk == initial_chunk:
		return _with_session_cleanup(false, "cross_chunk_boundary", move_step, {
			"expected_new_chunk": true,
			"initial_chunk": initial_chunk,
			"moved_chunk": moved_chunk,
		})

	var sample_step := await run_step("sample_height", {
		"scene_block": move_target,
	})
	if not _step_succeeded(sample_step):
		return _with_session_cleanup(false, "sample_height", sample_step)

	var screenshot_step := await run_step("capture_screenshot", {
		"file_name": "smoke",
	})
	var screenshot_result: Dictionary = screenshot_step.get("result", {})
	var screenshot_skipped := false
	if not _step_succeeded(screenshot_step):
		if String(screenshot_result.get("error_code", "")) != "headless_screenshot_unavailable":
			return _with_session_cleanup(false, "capture_screenshot", screenshot_step)
		screenshot_skipped = true

	var end_step := await run_step("end_session", {})
	if not _step_succeeded(end_step):
		return {
			"ok": false,
			"phase": "end_session",
			"step": AgentSessionScript.sanitize_variant(end_step),
		}

	return {
		"ok": true,
		"initial_chunk": AgentSessionScript.sanitize_variant(initial_chunk),
		"moved_chunk": AgentSessionScript.sanitize_variant(moved_chunk),
		"teleport_target": AgentSessionScript.sanitize_variant(teleport_land),
		"move_target": AgentSessionScript.sanitize_variant(move_target),
		"session": end_step.get("result", {}).get("data", {}).get("session", {}),
		"screenshot": screenshot_result.get("data", {}),
		"screenshot_skipped": screenshot_skipped,
		"screenshot_error_code": String(screenshot_result.get("error_code", "")),
	}

func _run_fly_swim_smoke_test() -> void:
	print("AgentRuntime: starting fly-swim smoke test")
	await get_tree().process_frame
	await get_tree().create_timer(0.5).timeout
	var result := await _execute_fly_swim_smoke_test()
	var ok := bool(result.get("ok", false))
	print("AgentRuntime fly-swim smoke test: %s" % ["PASS" if ok else "FAIL"])
	print(JSON.stringify(AgentSessionScript.sanitize_variant(result), "\t"))
	get_tree().quit(0 if ok else 1)

func _execute_fly_swim_smoke_test() -> Dictionary:
	var started := start_session("fly_swim_smoke_test", {
		"scenario": "fly_swim_smoke_test",
	})
	if String(started.get("result", {}).get("status", "")) != "completed":
		return {"ok": false, "phase": "start_session", "result": started}

	# ── Phase 1: verify initial state is WALKING ──────────────────────
	var state_step := await run_step("get_move_state", {})
	if not _step_succeeded(state_step):
		return _with_session_cleanup(false, "initial_get_move_state", state_step)
	var initial_state: String = state_step.get("result", {}).get("data", {}).get("move_state", "")
	if initial_state != "WALKING":
		return _with_session_cleanup(false, "initial_state_check", state_step, {
			"expected": "WALKING", "got": initial_state,
		})
	print("  [fly-swim] Initial state: %s ✓" % initial_state)

	# ── Phase 2: toggle fly, verify FLYING ────────────────────────────
	var fly_step := await run_step("toggle_fly", {})
	if not _step_succeeded(fly_step):
		return _with_session_cleanup(false, "toggle_fly_on", fly_step)

	state_step = await run_step("get_move_state", {})
	if not _step_succeeded(state_step):
		return _with_session_cleanup(false, "flying_get_move_state", state_step)
	var fly_state: String = state_step.get("result", {}).get("data", {}).get("move_state", "")
	if fly_state != "FLYING":
		return _with_session_cleanup(false, "fly_state_check", state_step, {
			"expected": "FLYING", "got": fly_state,
		})
	print("  [fly-swim] After toggle_fly: %s ✓" % fly_state)

	# ── Phase 3: wait in flight, verify no gravity (Y stays stable) ───
	var pre_fly_y: float = _player.position.y
	var wait_step := await run_step("wait_seconds", {
		"duration_seconds": 1.0,
		"timeout_seconds": 2.0,
	})
	if not _step_succeeded(wait_step):
		return _with_session_cleanup(false, "fly_wait", wait_step)
	var post_fly_y: float = _player.position.y
	var fly_y_drift := absf(post_fly_y - pre_fly_y)
	if fly_y_drift > 1.0:
		return _with_session_cleanup(false, "fly_no_gravity", wait_step, {
			"pre_fly_y": pre_fly_y, "post_fly_y": post_fly_y, "drift": fly_y_drift,
			"reason": "Player drifted %.1f units vertically while flying (expected < 1.0)" % fly_y_drift,
		})
	print("  [fly-swim] Flight Y drift: %.2f (< 1.0) ✓" % fly_y_drift)

	# ── Phase 3b: altitude gain (set_fly_vertical +1) ────────────────
	var ascend_step := await run_step("set_fly_vertical", {"vertical": 1.0})
	if not _step_succeeded(ascend_step):
		return _with_session_cleanup(false, "set_fly_vertical_ascend", ascend_step)
	var pre_ascend_y: float = _player.position.y
	wait_step = await run_step("wait_seconds", {
		"duration_seconds": 1.0,
		"timeout_seconds": 2.0,
	})
	if not _step_succeeded(wait_step):
		return _with_session_cleanup(false, "ascend_wait", wait_step)
	var post_ascend_y: float = _player.position.y
	var ascend_delta := post_ascend_y - pre_ascend_y
	if ascend_delta < 5.0:
		return _with_session_cleanup(false, "altitude_gain", wait_step, {
			"pre_y": pre_ascend_y, "post_y": post_ascend_y, "delta": ascend_delta,
			"reason": "Player gained only %.1f altitude (expected >= 5.0)" % ascend_delta,
		})
	print("  [fly-swim] Altitude gain: %.2f (>= 5.0) ✓" % ascend_delta)

	# ── Phase 3c: altitude loss (set_fly_vertical -1) ────────────────
	var descend_step := await run_step("set_fly_vertical", {"vertical": -1.0})
	if not _step_succeeded(descend_step):
		return _with_session_cleanup(false, "set_fly_vertical_descend", descend_step)
	var pre_descend_y: float = _player.position.y
	wait_step = await run_step("wait_seconds", {
		"duration_seconds": 1.0,
		"timeout_seconds": 2.0,
	})
	if not _step_succeeded(wait_step):
		return _with_session_cleanup(false, "descend_wait", wait_step)
	var post_descend_y: float = _player.position.y
	var descend_delta := pre_descend_y - post_descend_y
	if descend_delta < 5.0:
		return _with_session_cleanup(false, "altitude_loss", wait_step, {
			"pre_y": pre_descend_y, "post_y": post_descend_y, "delta": descend_delta,
			"reason": "Player lost only %.1f altitude (expected >= 5.0)" % descend_delta,
		})
	print("  [fly-swim] Altitude loss: %.2f (>= 5.0) ✓" % descend_delta)

	# Stop vertical movement before toggling fly off
	await run_step("set_fly_vertical", {"vertical": 0.0})

	# ── Phase 4: toggle fly off, verify back to WALKING ───────────────
	fly_step = await run_step("toggle_fly", {})
	if not _step_succeeded(fly_step):
		return _with_session_cleanup(false, "toggle_fly_off", fly_step)

	state_step = await run_step("get_move_state", {})
	if not _step_succeeded(state_step):
		return _with_session_cleanup(false, "walking_get_move_state", state_step)
	var walk_state: String = state_step.get("result", {}).get("data", {}).get("move_state", "")
	if walk_state != "WALKING":
		return _with_session_cleanup(false, "walk_state_check", state_step, {
			"expected": "WALKING", "got": walk_state,
		})
	print("  [fly-swim] After toggle_fly off: %s ✓" % walk_state)

	# ── Phase 5: teleport to water, verify SWIMMING ───────────────────
	var obs: Dictionary = started.get("observation", {})
	var blocks_per_chunk: int = int(obs.get("runtime_constants", {}).get("blocks_per_chunk", 512))
	var anchor := _dict_to_vector2i(obs.get("anchor_chunk", {}))
	var current := _dict_to_vector2i(obs.get("current_chunk", {}))
	var origin_x: int = (current.x - anchor.x) * blocks_per_chunk
	var origin_z: int = (current.y - anchor.y) * blocks_per_chunk
	var cx: int = origin_x + blocks_per_chunk / 2
	var cz: int = origin_z + blocks_per_chunk / 2

	var water_block: Vector2i = Vector2i(-1, -1)
	for ring in range(0, 384, 16):
		if water_block.x >= 0:
			break
		for dx in range(-ring, ring + 1, 16):
			for dz_off in ([-ring, ring] if ring > 0 else [0]):
				var bx: int = cx + dx
				var bz: int = cz + dz_off
				var sample_step := await run_step("sample_height", {"scene_block": Vector2i(bx, bz)})
				if _step_succeeded(sample_step):
					var h: float = float(sample_step.get("result", {}).get("data", {}).get("height", 999.0))
					if h < float(VoxelMeshBuilder.SEA_LEVEL_Y):
						water_block = Vector2i(bx, bz)
						break
			if water_block.x >= 0:
				break

	if water_block.x < 0:
		print("  [fly-swim] No water found near spawn — skipping swim test")
	else:
		print("  [fly-swim] Found water at (%d, %d)" % [water_block.x, water_block.y])
		var tp_step := await run_step("teleport_to_block", {"scene_block": water_block})
		if not _step_succeeded(tp_step):
			return _with_session_cleanup(false, "teleport_to_water", tp_step)

		await run_step("wait_seconds", {"duration_seconds": 1.0, "timeout_seconds": 2.0})

		state_step = await run_step("get_move_state", {})
		if _step_succeeded(state_step):
			var swim_state: String = state_step.get("result", {}).get("data", {}).get("move_state", "")
			var swim_y: float = _player.position.y
			var fall_from_sea: float = float(VoxelMeshBuilder.SEA_LEVEL_Y) - swim_y
			print("  [fly-swim] In water state: %s, Y: %.1f, fall: %.1f" % [swim_state, swim_y, fall_from_sea])
			if swim_state == "SWIMMING":
				print("  [fly-swim] Swimming state ✓")
			else:
				print("  [fly-swim] WARNING: expected SWIMMING, got %s" % swim_state)
			if fall_from_sea < 10.0:
				print("  [fly-swim] Buoyancy check ✓ (fall %.1f < 10.0)" % fall_from_sea)
			else:
				return _with_session_cleanup(false, "swim_buoyancy", state_step, {
					"fall_from_sea": fall_from_sea,
					"reason": "Player fell %.1f below sea level (expected < 10.0)" % fall_from_sea,
				})

	# ── Cleanup ───────────────────────────────────────────────────────
	var end_step := await run_step("end_session", {})
	if not _step_succeeded(end_step):
		return {"ok": false, "phase": "end_session", "step": AgentSessionScript.sanitize_variant(end_step)}

	return {
		"ok": true,
		"phases": ["initial_walking", "toggle_fly_on", "fly_no_gravity", "altitude_gain", "altitude_loss", "toggle_fly_off", "swim_test"],
		"water_found": water_block.x >= 0,
	}

func _with_session_cleanup(ok: bool, phase: String, step: Dictionary, extra: Dictionary = {}) -> Dictionary:
	var session_snapshot := current_session_summary()
	var end_data := end_session("smoke test cleanup after %s" % phase)
	return {
		"ok": ok,
		"phase": phase,
		"step": AgentSessionScript.sanitize_variant(step),
		"extra": AgentSessionScript.sanitize_variant(extra),
		"session_before_cleanup": AgentSessionScript.sanitize_variant(session_snapshot),
		"cleanup": AgentSessionScript.sanitize_variant(end_data),
	}

func _step_succeeded(step: Dictionary) -> bool:
	var result: Dictionary = step.get("result", {})
	return String(result.get("status", "")) == "completed"

func _step_scene_block(step: Dictionary, field_name: String) -> Vector2i:
	var result: Dictionary = step.get("result", {})
	var data: Dictionary = result.get("data", {})
	return _dict_to_vector2i(data.get(field_name, Vector2i.ZERO))

func _dict_to_vector2i(value) -> Vector2i:
	if value is Vector2i:
		return value
	if value is Dictionary:
		if value.has("x") and value.has("z"):
			return Vector2i(int(value["x"]), int(value["z"]))
		return Vector2i(int(value.get("x", 0)), int(value.get("y", 0)))
	if value is Vector2:
		return Vector2i(int(value.x), int(value.y))
	return Vector2i.ZERO

func _result(
		status: String,
		action_name: String,
		payload: Dictionary = {},
		error_code: String = "",
		reason: String = "",
		hint: String = "") -> Dictionary:
	var result := {
		"schema_version": 1,
		"session_id": _session.session_id if _session != null else "",
		"step_index": _session.step_count if _session != null else 0,
		"timestamp_ms": Time.get_ticks_msec(),
		"action": action_name,
		"status": status,
		"data": AgentSessionScript.sanitize_variant(payload),
	}
	if not error_code.is_empty():
		result["error_code"] = error_code
	if not reason.is_empty():
		result["reason"] = reason
	if not hint.is_empty():
		result["hint"] = hint
	return result
