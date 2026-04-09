extends Control

const MAIN_MENU_SCENE_PATH := "res://scenes/main_menu.tscn"
const WORLD_SCENE_PATH := "res://scenes/world.tscn"
const MACRO_WORLD_SIZE := Vector2(1024.0, 512.0)
const MIN_ZOOM := 0.1
const MAX_ZOOM := 8.0
const ZOOM_STEP := 1.25
const TRACKPAD_ZOOM_DIVISOR := 6.0
const DRAG_THRESHOLD := 6.0
const COMPARE_GRID := 8
const RUNTIME_CHUNK_PREVIEW_RENDERER := preload("res://scripts/ui/runtime_chunk_preview_renderer.gd")

@onready var _status_label: Label = $MarginContainer/VBoxContainer/StatusLabel
@onready var _selection_label: Label = $MarginContainer/VBoxContainer/SelectionLabel
@onready var _zoom_label: Label = $MarginContainer/VBoxContainer/ControlsRow/ZoomLabel
@onready var _launch_button: Button = $MarginContainer/VBoxContainer/ControlsRow/LaunchButton
@onready var _back_button: Button = $MarginContainer/VBoxContainer/ControlsRow/BackButton
@onready var _map_viewport: Control = $MarginContainer/VBoxContainer/ContentRow/MapFrame/MapViewport
@onready var _map_canvas: Control = $MarginContainer/VBoxContainer/ContentRow/MapFrame/MapViewport/MapCanvas
@onready var _map_texture_rect: TextureRect = $MarginContainer/VBoxContainer/ContentRow/MapFrame/MapViewport/MapCanvas/MapTexture
@onready var _hover_rect: ColorRect = $MarginContainer/VBoxContainer/ContentRow/MapFrame/MapViewport/MapCanvas/HoverRect
@onready var _selected_rect: ColorRect = $MarginContainer/VBoxContainer/ContentRow/MapFrame/MapViewport/MapCanvas/SelectedRect
@onready var _preview_status_label: Label = $MarginContainer/VBoxContainer/ContentRow/PreviewFrame/PreviewVBox/PreviewStatusLabel
@onready var _preview_texture_rect: TextureRect = $MarginContainer/VBoxContainer/ContentRow/PreviewFrame/PreviewVBox/PreviewTexture

var _macro_texture: Texture2D
var _macro_seed: int = 42
var _macro_size := Vector2.ONE
var _zoom := 1.0
var _fit_zoom := 1.0
var _map_origin := Vector2.ZERO
var _hover_chunk := Vector2i(-1, -1)
var _selected_chunk := Vector2i(-1, -1)
var _dragging := false
var _drag_moved := false
var _drag_start_position := Vector2.ZERO
var _drag_last_position := Vector2.ZERO
var _drag_started_on_chunk := Vector2i(-1, -1)
var _compare_mode := false
var _compare_button: Button
var _preview_renderer
var _preview_request_token := 0

func _ready() -> void:
	resized.connect(_on_selector_resized)
	_map_viewport.resized.connect(_on_selector_resized)
	var window := get_window()
	if window:
		window.size_changed.connect(_on_selector_resized)
		window.size_changed.connect(_on_window_size_changed)
	_launch_button.pressed.connect(_on_launch_button_pressed)
	_back_button.pressed.connect(_on_back_pressed)
	_compare_button = Button.new()
	_compare_button.text = "Compare Gen"
	_compare_button.pressed.connect(_on_compare_button_pressed)
	$MarginContainer/VBoxContainer/ControlsRow.add_child(_compare_button)
	_preview_renderer = RUNTIME_CHUNK_PREVIEW_RENDERER.new()
	add_child(_preview_renderer)
	_map_viewport.gui_input.connect(_on_map_viewport_gui_input)
	_map_viewport.mouse_exited.connect(_on_map_viewport_mouse_exited)
	_macro_texture = _load_macro_texture()
	_configure_map()
	call_deferred("_refresh_responsive_layout")
	var capture_dir := OS.get_environment("MGRIP_SELECTOR_CAPTURE_DIR")
	if not capture_dir.is_empty():
		call_deferred("_run_capture_probe", capture_dir)

func _on_back_pressed() -> void:
	get_tree().change_scene_to_file(MAIN_MENU_SCENE_PATH)

func _on_launch_button_pressed() -> void:
	if not _has_selection():
		return
	GameState.set_selected_chunk_launch(_selected_chunk)
	get_tree().change_scene_to_file(WORLD_SCENE_PATH)

func _input(event: InputEvent) -> void:
	_handle_input_event(event)

func _unhandled_input(event: InputEvent) -> void:
	_handle_input_event(event)

func _on_map_viewport_gui_input(event: InputEvent) -> void:
	_handle_input_event(event)

func _handle_input_event(event: InputEvent) -> void:
	if get_viewport().is_input_handled():
		return
	if _macro_texture == null:
		return

	var local_position: Variant = _map_viewport_local_position_for_event(event)
	var event_in_viewport: bool = local_position is Vector2
	if not event_in_viewport and not _dragging:
		return

	if event is InputEventMouseButton:
		var mouse_button := event as InputEventMouseButton
		if event_in_viewport and mouse_button.button_index == MOUSE_BUTTON_WHEEL_UP and mouse_button.pressed:
			_set_zoom(_zoom * ZOOM_STEP, local_position)
			get_viewport().set_input_as_handled()
			return
		if event_in_viewport and mouse_button.button_index == MOUSE_BUTTON_WHEEL_DOWN and mouse_button.pressed:
			_set_zoom(_zoom / ZOOM_STEP, local_position)
			get_viewport().set_input_as_handled()
			return
		if mouse_button.button_index == MOUSE_BUTTON_LEFT:
			if mouse_button.pressed and event_in_viewport:
				_dragging = true
				_drag_moved = false
				_drag_start_position = local_position
				_drag_last_position = local_position
				_drag_started_on_chunk = _chunk_from_viewport_pos(local_position)
				get_viewport().set_input_as_handled()
			elif not mouse_button.pressed and _dragging:
				if _dragging and not _drag_moved and _drag_started_on_chunk.x >= 0:
					if _compare_mode:
						_open_compare_view(_snap_to_meso(_drag_started_on_chunk))
					else:
						_set_selected_chunk(_drag_started_on_chunk)
				_dragging = false
				_drag_moved = false
				get_viewport().set_input_as_handled()
				return

	if event is InputEventMouseMotion:
		var motion := event as InputEventMouseMotion
		if event_in_viewport:
			_refresh_hover_chunk(local_position)
		if _dragging:
			_map_origin += motion.relative
			_drag_last_position += motion.relative
			if _drag_last_position.distance_to(_drag_start_position) >= DRAG_THRESHOLD:
				_drag_moved = true
			_apply_map_transform()
			get_viewport().set_input_as_handled()
			return

	if event is InputEventPanGesture and event_in_viewport:
		var pan := event as InputEventPanGesture
		var zoom_scale := pow(ZOOM_STEP, -pan.delta.y / TRACKPAD_ZOOM_DIVISOR)
		_set_zoom(_zoom * zoom_scale, local_position)
		get_viewport().set_input_as_handled()
		return

	if event is InputEventMagnifyGesture and event_in_viewport:
		var magnify := event as InputEventMagnifyGesture
		_set_zoom(_zoom * (1.0 + magnify.factor), local_position)
		get_viewport().set_input_as_handled()
		return

func _on_map_viewport_mouse_exited() -> void:
	_hover_chunk = Vector2i(-1, -1)
	_update_chunk_rects()
	_update_labels()

func _configure_map() -> void:
	_hover_rect.visible = false
	_selected_rect.visible = false
	_selected_chunk = Vector2i(-1, -1)
	_hover_chunk = Vector2i(-1, -1)
	_launch_button.disabled = true

	if _macro_texture == null:
		_map_texture_rect.texture = null
		_status_label.text = "Macro map unavailable. Generate world layers first, or use Quick Launch."
		_selection_label.text = "No map loaded."
		_set_preview_placeholder("Preview unavailable until a macro map and chunk selection exist.")
		zoom_label_set()
		return

	_map_texture_rect.texture = _macro_texture
	_map_texture_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_map_texture_rect.stretch_mode = TextureRect.STRETCH_SCALE
	_map_texture_rect.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	_macro_size = Vector2(_macro_texture.get_width(), _macro_texture.get_height())
	_status_label.text = "Scroll to zoom, click and drag to pan, click to select a chunk."
	_selection_label.text = "No chunk selected."
	_set_preview_placeholder("Select a chunk to render the true top-down runtime preview.")
	_refresh_responsive_layout(true)

func _set_zoom(new_zoom: float, focus_position: Vector2 = Vector2(-1.0, -1.0)) -> void:
	if _macro_texture == null:
		zoom_label_set()
		return

	var clamped_zoom := clampf(new_zoom, maxf(MIN_ZOOM, _fit_zoom), MAX_ZOOM)
	var old_scaled_size := _scaled_map_size()
	var old_origin := _map_origin
	_zoom = clamped_zoom

	if focus_position.x < 0.0 or old_scaled_size.x <= 0.0 or old_scaled_size.y <= 0.0:
		_map_origin = _centered_origin_for_size(_scaled_map_size())
		_apply_map_transform()
		return

	var focus_uv := Vector2(
		(focus_position.x - old_origin.x) / old_scaled_size.x,
		(focus_position.y - old_origin.y) / old_scaled_size.y,
	)
	var new_scaled_size := _scaled_map_size()
	_map_origin = focus_position - Vector2(
		focus_uv.x * new_scaled_size.x,
		focus_uv.y * new_scaled_size.y,
	)
	_apply_map_transform()
	_refresh_hover_chunk(focus_position)

func _on_selector_resized() -> void:
	call_deferred("_refresh_responsive_layout")

func _on_window_size_changed() -> void:
	var window := get_window()
	if window:
		print("map_selector window_size=", window.size)

func _refresh_responsive_layout(force_fit: bool = false) -> void:
	if _macro_texture == null:
		return

	var viewport_size := _map_viewport.size
	if viewport_size.x <= 0.0 or viewport_size.y <= 0.0:
		return

	var previous_fit_zoom := _fit_zoom
	_fit_zoom = _fit_zoom_for_viewport_size(viewport_size)
	if force_fit or is_equal_approx(_zoom, previous_fit_zoom) or _zoom < _fit_zoom:
		_zoom = _fit_zoom
		_map_origin = _centered_origin_for_size(_scaled_map_size())
	else:
		_map_origin = _clamp_map_origin(_map_origin)
	_apply_map_transform()

func _apply_map_transform() -> void:
	var scaled_size := _scaled_map_size()
	_map_origin = _clamp_map_origin(_map_origin)
	_map_canvas.position = _map_origin.round()
	_map_canvas.custom_minimum_size = scaled_size
	_map_canvas.size = scaled_size
	_map_texture_rect.position = Vector2.ZERO
	_map_texture_rect.size = scaled_size
	_refresh_hover_from_mouse()
	_update_chunk_rects()
	zoom_label_set()

func _fit_zoom_for_viewport_size(viewport_size: Vector2) -> float:
	var fit_zoom := minf(
		viewport_size.x / _macro_size.x,
		viewport_size.y / _macro_size.y,
	)
	return clampf(fit_zoom, MIN_ZOOM, MAX_ZOOM)

func _scaled_map_size() -> Vector2:
	return _macro_size * _zoom

func _centered_origin_for_size(scaled_size: Vector2) -> Vector2:
	return (_map_viewport.size - scaled_size) * 0.5

func _clamp_map_origin(origin: Vector2) -> Vector2:
	var scaled_size := _scaled_map_size()
	var viewport_size := _map_viewport.size
	var clamped := origin
	if scaled_size.x <= viewport_size.x:
		clamped.x = (viewport_size.x - scaled_size.x) * 0.5
	else:
		clamped.x = clampf(origin.x, viewport_size.x - scaled_size.x, 0.0)
	if scaled_size.y <= viewport_size.y:
		clamped.y = (viewport_size.y - scaled_size.y) * 0.5
	else:
		clamped.y = clampf(origin.y, viewport_size.y - scaled_size.y, 0.0)
	return clamped

func _set_selected_chunk(chunk_coord: Vector2i) -> void:
	_selected_chunk = chunk_coord
	_launch_button.disabled = false
	_update_chunk_rects()
	_update_labels()
	_request_preview_for_chunk(chunk_coord)

func _snap_to_meso(chunk: Vector2i) -> Vector2i:
	return Vector2i(
		(chunk.x / COMPARE_GRID) * COMPARE_GRID,
		(chunk.y / COMPARE_GRID) * COMPARE_GRID,
	)

func _update_labels() -> void:
	var hover_text: String
	if _hover_chunk.x < 0:
		hover_text = "Hover: none"
	elif _compare_mode:
		var mx := _hover_chunk.x / COMPARE_GRID
		var my := _hover_chunk.y / COMPARE_GRID
		hover_text = "Meso: (%d, %d)" % [mx, my]
	else:
		hover_text = "Hover: (%d, %d)" % [_hover_chunk.x, _hover_chunk.y]

	var selected_text := "Selected: (%d, %d)" % [_selected_chunk.x, _selected_chunk.y] if _has_selection() else "Selected: none"
	_selection_label.text = "%s    %s" % [hover_text, selected_text]

func _refresh_hover_chunk(viewport_position: Vector2) -> void:
	var hover_chunk := _chunk_from_viewport_pos(viewport_position)
	if hover_chunk.x >= 0 and _compare_mode:
		hover_chunk = _snap_to_meso(hover_chunk)
	if hover_chunk == _hover_chunk:
		return
	_hover_chunk = hover_chunk
	_update_chunk_rects()
	_update_labels()

func _refresh_hover_from_mouse() -> void:
	var mouse_local_position: Variant = _map_viewport_local_position_from_mouse()
	if mouse_local_position == null:
		return
	_refresh_hover_chunk(mouse_local_position)

func _update_chunk_rects() -> void:
	if _hover_chunk.x >= 0:
		_hover_rect.visible = true
		var r := _marker_rect_for_chunk(_hover_chunk, COMPARE_GRID if _compare_mode else 1)
		_hover_rect.position = r.position
		_hover_rect.size = r.size
	else:
		_hover_rect.visible = false

	if _has_selection():
		_selected_rect.visible = true
		var r := _marker_rect_for_chunk(_selected_chunk, COMPARE_GRID if _compare_mode else 1)
		_selected_rect.position = r.position
		_selected_rect.size = r.size
	else:
		_selected_rect.visible = false

func _chunk_size_in_canvas() -> Vector2:
	return Vector2(
		_map_canvas.size.x / MACRO_WORLD_SIZE.x,
		_map_canvas.size.y / MACRO_WORLD_SIZE.y,
	)

func _marker_rect_for_chunk(chunk_coord: Vector2i, grid: int = 1) -> Rect2:
	var cell_size := _chunk_size_in_canvas()
	var chunk_origin := Vector2(chunk_coord.x, chunk_coord.y) * cell_size
	return Rect2(chunk_origin, cell_size * grid)

func _chunk_from_viewport_pos(pos: Vector2) -> Vector2i:
	if _macro_texture == null:
		return Vector2i(-1, -1)
	var local := pos - _map_canvas.position
	if local.x < 0.0 or local.y < 0.0 or local.x >= _map_canvas.size.x or local.y >= _map_canvas.size.y:
		return Vector2i(-1, -1)
	var world_x := clampf(local.x / _map_canvas.size.x * MACRO_WORLD_SIZE.x, 0.0, MACRO_WORLD_SIZE.x - 0.001)
	var world_y := clampf(local.y / _map_canvas.size.y * MACRO_WORLD_SIZE.y, 0.0, MACRO_WORLD_SIZE.y - 0.001)
	return Vector2i(int(floor(world_x)), int(floor(world_y)))

func _has_selection() -> bool:
	return _selected_chunk.x >= 0 and _selected_chunk.y >= 0

func _map_viewport_local_position_for_event(event: InputEvent) -> Variant:
	if (
		event is InputEventMouseButton
		or event is InputEventMouseMotion
		or event is InputEventPanGesture
		or event is InputEventMagnifyGesture
	):
		return _map_viewport_local_position_from_mouse()
	return null

func _map_viewport_local_position_from_mouse() -> Variant:
	var local_position := _map_viewport.get_local_mouse_position()
	if (
		local_position.x < 0.0
		or local_position.y < 0.0
		or local_position.x > _map_viewport.size.x
		or local_position.y > _map_viewport.size.y
	):
		return null
	return local_position

func _run_capture_probe(capture_dir: String) -> void:
	DirAccess.make_dir_recursive_absolute(capture_dir)
	await get_tree().process_frame
	await get_tree().process_frame
	await get_tree().process_frame
	_save_viewport_capture(capture_dir.path_join("selector_initial.png"))

	DisplayServer.window_set_size(Vector2i(1100, 700))
	await get_tree().process_frame
	await get_tree().process_frame
	_save_viewport_capture(capture_dir.path_join("selector_resized.png"))

	var center := _map_viewport.size * 0.5
	var pan_zoom := InputEventPanGesture.new()
	pan_zoom.position = center
	pan_zoom.delta = Vector2(0.0, -36.0)
	_input(pan_zoom)
	await get_tree().process_frame

	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	press.pressed = true
	press.position = center
	_input(press)

	var drag := InputEventMouseMotion.new()
	drag.position = center + Vector2(180.0, 60.0)
	drag.relative = Vector2(180.0, 60.0)
	_input(drag)

	var release := InputEventMouseButton.new()
	release.button_index = MOUSE_BUTTON_LEFT
	release.pressed = false
	release.position = center + Vector2(180.0, 60.0)
	_input(release)
	await get_tree().process_frame
	_save_viewport_capture(capture_dir.path_join("selector_zoom_pan.png"))
	print("selector_probe viewport=", _map_viewport.size)
	print("selector_probe canvas=", _map_canvas.size)
	print("selector_probe origin=", _map_canvas.position)
	print("selector_probe zoom=", _zoom)

	_set_selected_chunk(Vector2i(400, 250))
	await get_tree().create_timer(1.0).timeout
	_save_viewport_capture(capture_dir.path_join("selector_preview.png"))

	# ── Compare Generation probe ─────────────────────────────────────────────
	if _macro_texture == null:
		print("selector_probe compare SKIP: no macro texture")
		get_tree().quit()
		return

	_on_compare_button_pressed()
	await get_tree().process_frame
	_save_viewport_capture(capture_dir.path_join("compare_mode_entered.png"))

	_open_compare_view(Vector2i(192, 248))  # meso (24,31)
	await get_tree().process_frame
	_save_viewport_capture(capture_dir.path_join("compare_opened.png"))

	# Compare generation now renders one continuous 8x8 LOD0 region preview.
	await get_tree().create_timer(24.0).timeout
	_save_viewport_capture(capture_dir.path_join("compare_result.png"))

	# Find the view and print its agreement label text
	for child in get_tree().current_scene.get_children():
		if child is CanvasLayer:
			for sub in child.get_children():
				if sub.has_method("show_comparison"):
					print("compare_probe agreement=", sub._agreement_label.text)
					print("compare_probe status=", sub._status_label.text)

	get_tree().quit()

func _save_viewport_capture(path: String) -> void:
	var viewport_texture := get_viewport().get_texture()
	if viewport_texture == null:
		print("selector_probe capture SKIP: viewport texture unavailable")
		return
	var image := viewport_texture.get_image()
	if image == null:
		print("selector_probe capture SKIP: viewport image unavailable")
		return
	image.save_png(path)

func _load_macro_texture() -> Texture2D:
	var home := OS.get_environment("HOME")
	if home.is_empty():
		return null
	var layers_dir := home.path_join(".margins_grip/layers")
	var dir := DirAccess.open(layers_dir)
	if dir == null:
		return null

	var newest_path := ""
	var newest_entry_dir := ""
	var newest_time := -1
	for entry in dir.get_directories():
		var image_path := layers_dir.path_join(entry).path_join("images/biome.png")
		if not FileAccess.file_exists(image_path):
			continue
		var mtime := FileAccess.get_modified_time(image_path)
		if mtime > newest_time:
			newest_time = mtime
			newest_path = image_path
			newest_entry_dir = layers_dir.path_join(entry)

	if newest_path.is_empty():
		return null

	# Parse seed from manifest.ron if present
	var manifest_path := newest_entry_dir.path_join("manifest.ron")
	if FileAccess.file_exists(manifest_path):
		var manifest_text := FileAccess.get_file_as_string(manifest_path)
		var regex := RegEx.new()
		regex.compile("seed:\\s*(\\d+)")
		var rx_match := regex.search(manifest_text)
		if rx_match:
			_macro_seed = int(rx_match.get_string(1))

	var image := Image.load_from_file(newest_path)
	if image == null:
		return null
	return ImageTexture.create_from_image(image)

func _on_compare_button_pressed() -> void:
	_compare_mode = not _compare_mode
	_compare_button.text = "Exit Compare" if _compare_mode else "Compare Gen"
	_status_label.text = (
		"Compare mode: click a chunk to open the 3-panel comparison with rendered runtime previews."
		if _compare_mode
		else "Scroll to zoom, click and drag to pan, click to select a chunk."
	)

func _open_compare_view(chunk: Vector2i) -> void:
	var layer := CanvasLayer.new()
	layer.layer = 128
	get_tree().current_scene.add_child(layer)
	var view := preload("res://scripts/ui/compare_generation_view.gd").new()
	layer.add_child(view)
	view.show_comparison(_macro_seed, float(chunk.x), float(chunk.y), COMPARE_GRID, _macro_texture, MACRO_WORLD_SIZE)

func zoom_label_set() -> void:
	_zoom_label.text = "Zoom: %.2fx" % _zoom


func _set_preview_placeholder(message: String) -> void:
	_preview_texture_rect.texture = null
	_preview_status_label.text = message


func _request_preview_for_chunk(chunk_coord: Vector2i) -> void:
	_preview_request_token += 1
	var token := _preview_request_token
	_set_preview_placeholder(
		"Rendering traversed-terrain preview for chunk (%d, %d)…" % [chunk_coord.x, chunk_coord.y]
	)
	call_deferred("_render_chunk_preview_async", chunk_coord, token)


func _render_chunk_preview_async(chunk_coord: Vector2i, token: int) -> void:
	var preview: Dictionary = _preview_renderer.render_chunk_preview(_macro_seed, chunk_coord)
	if not is_inside_tree() or token != _preview_request_token:
		return
	_preview_texture_rect.texture = preview.get("texture")
	_preview_status_label.text = _preview_summary_text(chunk_coord, preview.get("summary", {}))


func _preview_summary_text(chunk_coord: Vector2i, summary: Dictionary) -> String:
	var zone: Dictionary = summary.get("planet_zone", {})
	var water: Dictionary = summary.get("water_state", {})
	var landform: Dictionary = summary.get("landform_class", {})
	return "Chunk (%d, %d)\n%s / %s / %s" % [
		chunk_coord.x,
		chunk_coord.y,
		String(zone.get("name", "Unknown Zone")),
		String(water.get("name", "Unknown Water")),
		String(landform.get("name", "Unknown Landform")),
	]
