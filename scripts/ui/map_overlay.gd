extends CanvasLayer
class_name MapOverlay

const LOCAL_MAP_SIZE := Vector2(760.0, 760.0)
const MACRO_WORLD_SIZE := Vector2(1024.0, 512.0)

enum MapMode { LOCAL, MACRO }

var _anchor_chunk: Vector2i = Vector2i.ZERO
var _local_chunk: Vector2i = Vector2i.ZERO
var _mode: MapMode = MapMode.LOCAL

var _bg: ColorRect
var _map_rect: TextureRect
var _marker: ColorRect
var _title: Label
var _map_label: Label
var _hint_label: Label
var _hud: Label

var _local_texture: Texture2D
var _macro_texture: Texture2D
var _macro_size := Vector2.ONE

func _ready() -> void:
	layer = 10
	visible = false

	_bg = ColorRect.new()
	_bg.color = Color(0.0, 0.0, 0.0, 0.78)
	_bg.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(_bg)

	_map_rect = TextureRect.new()
	_map_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_map_rect.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	add_child(_map_rect)

	_marker = ColorRect.new()
	_marker.color = Color(1.0, 0.15, 0.15)
	_marker.size = Vector2(10.0, 10.0)
	add_child(_marker)

	_title = Label.new()
	_title.add_theme_font_size_override("font_size", 18)
	_title.add_theme_color_override("font_color", Color(0.92, 0.92, 0.92))
	add_child(_title)

	_map_label = Label.new()
	_map_label.add_theme_font_size_override("font_size", 15)
	_map_label.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0))
	add_child(_map_label)

	_hint_label = Label.new()
	_hint_label.add_theme_font_size_override("font_size", 14)
	_hint_label.add_theme_color_override("font_color", Color(0.80, 0.80, 0.80))
	add_child(_hint_label)

func setup(biome_map: MgBiomeMap, anchor_chunk: Vector2i, local_chunk: Vector2i) -> void:
	_anchor_chunk = anchor_chunk
	update_local_chunk(biome_map, local_chunk)
	_macro_texture = _load_macro_texture()
	_map_rect.texture = _local_texture

func update_local_chunk(biome_map: MgBiomeMap, local_chunk: Vector2i) -> void:
	_local_chunk = local_chunk
	var rgba := biome_map.export_layer_rgba("biome")
	var img  := Image.create_from_data(512, 512, false, Image.FORMAT_RGBA8, rgba)
	_local_texture = ImageTexture.create_from_image(img)

func refresh(player_pos: Vector3, current_chunk: Vector2i, active_counts: Dictionary = {}) -> void:
	if _hud:
		_hud.text = _coord_text(player_pos, current_chunk, active_counts)
	if not visible:
		return
	_layout(player_pos, current_chunk)

func toggle() -> void:
	visible = not visible

func toggle_mode() -> void:
	if _macro_texture == null:
		return
	_mode = MapMode.MACRO if _mode == MapMode.LOCAL else MapMode.LOCAL

func attach_hud(root: Node) -> void:
	var cl := CanvasLayer.new()
	cl.layer = 5
	_hud = Label.new()
	_hud.position = Vector2(12.0, 12.0)
	_hud.add_theme_font_size_override("font_size", 15)
	_hud.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0))
	_hud.add_theme_color_override("font_shadow_color", Color(0.0, 0.0, 0.0, 0.9))
	_hud.add_theme_constant_override("shadow_offset_x", 1)
	_hud.add_theme_constant_override("shadow_offset_y", 1)
	cl.add_child(_hud)
	root.add_child(cl)

func _layout(player_pos: Vector3, current_chunk: Vector2i) -> void:
	var vp := get_viewport().get_visible_rect().size
	var map_size := _active_map_size()
	var orig := (vp - map_size) * 0.5

	_map_rect.texture = _active_texture()
	_map_rect.position = orig
	_map_rect.size = map_size
	_map_rect.stretch_mode = (
		TextureRect.STRETCH_KEEP_ASPECT_CENTERED
		if _mode == MapMode.MACRO
		else TextureRect.STRETCH_SCALE
	)

	var marker_pos := _marker_position(player_pos, current_chunk, orig, map_size)
	_marker.position = marker_pos - _marker.size * 0.5

	_title.position = orig - Vector2(0.0, 28.0)
	_title.text = "%s MAP   [M] close   [Tab] switch" % _mode_name()
	_hint_label.position = orig + Vector2(0.0, map_size.y + 8.0)
	_hint_label.text = (
		"Tab switches Local/Macro map."
		if _macro_texture
		else "Macro map unavailable: generate world layers first."
	)
	_map_label.position = orig + Vector2(0.0, map_size.y + 30.0)
	_map_label.text = _mode_text(player_pos, current_chunk)

func _coord_text(p: Vector3, current_chunk: Vector2i, active_counts: Dictionary) -> String:
	var local_block := GenerationManager.scene_block_to_local_block(p.x, p.z)
	var world_origin := GenerationManager.chunk_coord_to_world_origin(current_chunk)
	var wx := world_origin.x + float(local_block.x) / float(GenerationManager.BLOCKS_PER_CHUNK)
	var wz := world_origin.y + float(local_block.y) / float(GenerationManager.BLOCKS_PER_CHUNK)
	return "Chunk (%d, %d)   Block (%d, %d)   Y: %d   Active %s   World (%.3f, %.3f)" % [
		current_chunk.x,
		current_chunk.y,
		local_block.x,
		local_block.y,
		int(p.y),
		_format_active_counts(active_counts),
		wx,
		wz,
	]

func _mode_text(player_pos: Vector3, current_chunk: Vector2i) -> String:
	var local_block := GenerationManager.scene_block_to_local_block(player_pos.x, player_pos.z)
	var world_origin := GenerationManager.chunk_coord_to_world_origin(current_chunk)
	var wx := world_origin.x + float(local_block.x) / float(GenerationManager.BLOCKS_PER_CHUNK)
	var wz := world_origin.y + float(local_block.y) / float(GenerationManager.BLOCKS_PER_CHUNK)
	if _mode == MapMode.LOCAL:
		return "Local block (%d, %d)   chunk (%d, %d)" % [
			local_block.x,
			local_block.y,
			current_chunk.x,
			current_chunk.y,
		]
	return "Macro world (%.3f, %.3f)   chunk (%d, %d)" % [wx, wz, current_chunk.x, current_chunk.y]

func _marker_position(
		player_pos: Vector3,
		current_chunk: Vector2i,
		origin: Vector2,
		map_size: Vector2) -> Vector2:
	if _mode == MapMode.LOCAL:
		var local_block := GenerationManager.scene_block_to_local_block(player_pos.x, player_pos.z)
		return origin + Vector2(
			float(local_block.x) / float(GenerationManager.BLOCKS_PER_CHUNK - 1),
			float(local_block.y) / float(GenerationManager.BLOCKS_PER_CHUNK - 1)
		) * map_size

	var local_block := GenerationManager.scene_block_to_local_block(player_pos.x, player_pos.z)
	var world_origin := GenerationManager.chunk_coord_to_world_origin(current_chunk)
	var wx := clampf(
		world_origin.x + float(local_block.x) / float(GenerationManager.BLOCKS_PER_CHUNK),
		0.0,
		MACRO_WORLD_SIZE.x
	)
	var wz := clampf(
		world_origin.y + float(local_block.y) / float(GenerationManager.BLOCKS_PER_CHUNK),
		0.0,
		MACRO_WORLD_SIZE.y
	)
	return origin + Vector2(wx / MACRO_WORLD_SIZE.x, wz / MACRO_WORLD_SIZE.y) * map_size

func _active_texture() -> Texture2D:
	if _mode == MapMode.MACRO and _macro_texture:
		return _macro_texture
	return _local_texture

func _active_map_size() -> Vector2:
	if _mode == MapMode.MACRO and _macro_texture:
		var vp := get_viewport().get_visible_rect().size
		var aspect := _macro_size.x / _macro_size.y
		var width := minf(vp.x * 0.78, 1100.0)
		var height := width / aspect
		var max_height := vp.y * 0.72
		if height > max_height:
			height = max_height
			width = height * aspect
		return Vector2(width, height)
	return LOCAL_MAP_SIZE

func _mode_name() -> String:
	return "LOCAL" if _mode == MapMode.LOCAL else "MACRO"

func _format_active_counts(active_counts: Dictionary) -> String:
	if active_counts.is_empty():
		return "{}"
	var keys := active_counts.keys()
	keys.sort()
	var parts: Array[String] = []
	for key in keys:
		parts.append("%s:%d" % [String(key), int(active_counts[key])])
	return "{%s}" % ", ".join(parts)

func _load_macro_texture() -> Texture2D:
	var home := OS.get_environment("HOME")
	if home.is_empty():
		return null
	var layers_dir := home.path_join(".margins_grip/layers")
	var dir := DirAccess.open(layers_dir)
	if dir == null:
		return null

	var newest_path := ""
	var newest_time := -1
	for entry in dir.get_directories():
		var image_path := layers_dir.path_join(entry).path_join("images/biome.png")
		if not FileAccess.file_exists(image_path):
			continue
		var mtime := FileAccess.get_modified_time(image_path)
		if mtime > newest_time:
			newest_time = mtime
			newest_path = image_path

	if newest_path.is_empty():
		return null

	var image := Image.load_from_file(newest_path)
	if image == null:
		return null
	_macro_size = Vector2(image.get_width(), image.get_height())
	return ImageTexture.create_from_image(image)
