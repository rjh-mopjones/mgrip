extends RefCounted
class_name ChunkMetrics

const SUMMARY_INTERVAL_MSEC := 3000
const HISTORY_LIMIT := 24
const ACTIVATION_SPIKE_WARN_MS := 50.0
const ACTIVATION_STALL_FAIL_MS := 150.0

var _last_summary_msec := 0
var _recent_total_ms: Array[float] = []
var _window_peak_ms := 0.0
var _active_by_lod: Dictionary = {}
var _pending_chunks := 0
var _horizon_state: Dictionary = {}

func begin_activation(chunk_coord: Vector2i, lod: String) -> Dictionary:
	return {
		"chunk_coord": chunk_coord,
		"lod": lod,
		"started_usec": Time.get_ticks_usec(),
		"generation_ms": 0.0,
		"mesh_ms": 0.0,
		"collision_ms": 0.0,
		"attach_ms": 0.0,
		"total_ms": 0.0,
	}

func set_phase_ms(sample: Dictionary, field: String, duration_ms: float) -> void:
	sample[field] = duration_ms

func finish_activation(sample: Dictionary) -> void:
	var total_ms := (Time.get_ticks_usec() - int(sample["started_usec"])) / 1000.0
	sample["total_ms"] = total_ms
	_recent_total_ms.append(total_ms)
	if _recent_total_ms.size() > HISTORY_LIMIT:
		_recent_total_ms.pop_front()
	_window_peak_ms = maxf(_window_peak_ms, total_ms)
	var coord := sample["chunk_coord"] as Vector2i
	print(
		"Chunk [%d, %d] %s  gen %.1fms  mesh %.1fms  collision %.1fms  attach %.1fms  total %.1fms"
		% [
			coord.x,
			coord.y,
			String(sample["lod"]),
			float(sample["generation_ms"]),
			float(sample["mesh_ms"]),
			float(sample["collision_ms"]),
			float(sample["attach_ms"]),
			total_ms,
		]
	)
	if total_ms >= ACTIVATION_STALL_FAIL_MS:
		print(
			"Chunk activation budget FAIL  [%d, %d] %s total=%.1fms exceeds %.1fms"
			% [coord.x, coord.y, String(sample["lod"]), total_ms, ACTIVATION_STALL_FAIL_MS]
		)
	elif total_ms >= ACTIVATION_SPIKE_WARN_MS:
		print(
			"Chunk activation budget WARN  [%d, %d] %s total=%.1fms exceeds %.1fms"
			% [coord.x, coord.y, String(sample["lod"]), total_ms, ACTIVATION_SPIKE_WARN_MS]
		)

func update_runtime_state(active_by_lod: Dictionary, pending_chunks: int) -> void:
	_active_by_lod = active_by_lod.duplicate(true)
	_pending_chunks = pending_chunks

func set_horizon_state(horizon_state: Dictionary) -> void:
	_horizon_state = horizon_state.duplicate(true)

func maybe_print_summary() -> void:
	var now := Time.get_ticks_msec()
	if _last_summary_msec == 0:
		_last_summary_msec = now
		return
	if now - _last_summary_msec < SUMMARY_INTERVAL_MSEC:
		return
	_last_summary_msec = now
	var avg_ms := 0.0
	for total_ms in _recent_total_ms:
		avg_ms += total_ms
	if not _recent_total_ms.is_empty():
		avg_ms /= float(_recent_total_ms.size())
	print(
		"Chunk runtime summary  active=%s  pending=%d  avg_total=%.1fms  peak_total=%.1fms%s%s"
		% [
			_format_active_counts(),
			_pending_chunks,
			avg_ms,
			_window_peak_ms,
			_format_horizon_state(),
			_format_budget_state(avg_ms),
		]
	)
	_window_peak_ms = 0.0

func _format_active_counts() -> String:
	if _active_by_lod.is_empty():
		return "{}"
	var keys := _active_by_lod.keys()
	keys.sort()
	var parts: Array[String] = []
	for key in keys:
		parts.append("%s:%d" % [String(key), int(_active_by_lod[key])])
	return "{%s}" % ", ".join(parts)

func _format_horizon_state() -> String:
	if _horizon_state.is_empty():
		return ""
	var focus: Vector2 = _horizon_state.get("focus", Vector2.ZERO)
	return (
		"  horizon={focus:(%.2f, %.2f) mid:%d/%d@r%d far:%d/%d@r%d}"
		% [
			focus.x,
			focus.y,
			int(_horizon_state.get("mid_loaded", 0)),
			int(_horizon_state.get("mid_budget", 0)),
			int(_horizon_state.get("mid_radius", 0)),
			int(_horizon_state.get("far_loaded", 0)),
			int(_horizon_state.get("far_budget", 0)),
			int(_horizon_state.get("far_radius", 0)),
		]
	)

func _format_budget_state(avg_ms: float) -> String:
	if _window_peak_ms >= ACTIVATION_STALL_FAIL_MS:
		return "  budget=FAIL(peak>=150ms)"
	if _window_peak_ms >= ACTIVATION_SPIKE_WARN_MS:
		return "  budget=WARN(peak>=50ms)"
	if avg_ms >= ACTIVATION_SPIKE_WARN_MS:
		return "  budget=WARN(avg>=50ms)"
	return "  budget=OK"
