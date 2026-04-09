extends Control

## Three-panel comparison tool.
## Left: macro biome.png crop for region context.
## Middle: true top-down renders of the real LOD0 runtime chunks.
## Right: semantic agreement overlay derived from macro/runtime chunk data.

const CELL_PX := 128
const MACRO_SAMPLE_RES := 65
const AGREE_COLOR := Color(0.12, 0.78, 0.31, 1.0)
const DISAGREE_COLOR := Color(0.82, 0.18, 0.18, 1.0)
const RUNTIME_CHUNK_PREVIEW_RENDERER := preload("res://scripts/ui/runtime_chunk_preview_renderer.gd")

var _seed: int
var _origin: Vector2
var _grid_size: int
var _macro_texture: Texture2D
var _macro_world_size: Vector2

var _title_label: Label
var _status_label: Label
var _macro_rect: TextureRect
var _micro_rect: TextureRect
var _diff_rect: TextureRect
var _agreement_label: Label
var _preview_renderer
var _generator := MgTerrainGen.new()
var _macro_ocean_cache: Dictionary = {}


func _ready() -> void:
	var vp_size := get_viewport().get_visible_rect().size
	position = Vector2.ZERO
	size = vp_size
	mouse_filter = Control.MOUSE_FILTER_STOP

	_preview_renderer = RUNTIME_CHUNK_PREVIEW_RENDERER.new()
	add_child(_preview_renderer)

	var bg := ColorRect.new()
	bg.position = Vector2.ZERO
	bg.size = vp_size
	bg.color = Color(0.08, 0.08, 0.10, 0.96)
	add_child(bg)

	var vbox := VBoxContainer.new()
	vbox.position = Vector2.ZERO
	vbox.size = vp_size
	vbox.add_theme_constant_override("separation", 8)
	add_child(vbox)

	_title_label = Label.new()
	_title_label.text = "Compare Generation"
	_title_label.add_theme_font_size_override("font_size", 18)
	_title_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	vbox.add_child(_title_label)

	_status_label = Label.new()
	_status_label.text = "Initialising…"
	_status_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_status_label.autowrap_mode = TextServer.AUTOWRAP_WORD
	vbox.add_child(_status_label)

	var panels_row := HBoxContainer.new()
	panels_row.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panels_row.add_theme_constant_override("separation", 8)
	vbox.add_child(panels_row)

	for panel_label in ["Macro  (biome.png)", "Runtime Preview  (LOD0)", "Diff"]:
		var col := VBoxContainer.new()
		col.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		var lbl := Label.new()
		lbl.text = panel_label
		lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		col.add_child(lbl)
		var rect := TextureRect.new()
		rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		rect.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
		rect.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		rect.size_flags_vertical = Control.SIZE_EXPAND_FILL
		rect.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
		col.add_child(rect)
		panels_row.add_child(col)
		match panel_label:
			"Macro  (biome.png)":
				_macro_rect = rect
			"Runtime Preview  (LOD0)":
				_micro_rect = rect
			"Diff":
				_diff_rect = rect

	_agreement_label = Label.new()
	_agreement_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_agreement_label.add_theme_font_size_override("font_size", 16)
	vbox.add_child(_agreement_label)

	var close_btn := Button.new()
	close_btn.text = "Close"
	close_btn.pressed.connect(func(): get_parent().queue_free())
	vbox.add_child(close_btn)


func show_comparison(
	seed: int,
	wx: float,
	wy: float,
	grid_size: int,
	macro_tex: Texture2D,
	macro_world: Vector2
) -> void:
	_seed = seed
	_origin = Vector2(wx, wy)
	_grid_size = grid_size
	_macro_texture = macro_tex
	_macro_world_size = macro_world
	var mx := int(wx) / grid_size
	var my := int(wy) / grid_size
	_title_label.text = "Compare Generation — Meso (%d, %d)" % [mx, my]
	_status_label.text = (
		"Rendering %dx%d true runtime previews for meso (%d, %d)…"
		% [grid_size, grid_size, mx, my]
	)
	_agreement_label.text = ""
	call_deferred("_generate")


func _generate() -> void:
	var n := _grid_size
	var img_w := n * CELL_PX
	var img_h := n * CELL_PX
	var macro_img := _build_macro_crop(n, img_w, img_h)
	_macro_rect.texture = ImageTexture.create_from_image(macro_img)

	var diff_img := Image.create(img_w, img_h, false, Image.FORMAT_RGBA8)
	_status_label.text = "Building %dx%d LOD0 local-map region from runtime chunk data…" % [n, n]
	var region_preview: Dictionary = _preview_renderer.render_chunk_grid_preview(
		_seed,
		Vector2i(int(_origin.x), int(_origin.y)),
		n
	)
	var region_image: Image = region_preview.get("image")
	region_image.resize(img_w, img_h, Image.INTERPOLATE_NEAREST)
	_micro_rect.texture = ImageTexture.create_from_image(region_image)

	var cell_ocean: Dictionary = region_preview.get("cell_ocean", {})
	var agree_count := 0
	var total := n * n
	for gy in range(n):
		for gx in range(n):
			var chunk_coord := Vector2i(int(_origin.x) + gx, int(_origin.y) + gy)
			var macro_ocean := _sample_macro_ocean(chunk_coord)
			var micro_ocean := bool(cell_ocean.get("%d:%d" % [chunk_coord.x, chunk_coord.y], false))
			var agree := macro_ocean == micro_ocean
			if agree:
				agree_count += 1

			diff_img.fill_rect(
				Rect2i(gx * CELL_PX, gy * CELL_PX, CELL_PX, CELL_PX),
				AGREE_COLOR if agree else DISAGREE_COLOR
			)
	_diff_rect.texture = ImageTexture.create_from_image(diff_img)

	var pct := 100.0 * agree_count / total
	_agreement_label.text = "Agreement: %d/%d  (%.1f%%)" % [agree_count, total, pct]
	_status_label.text = (
		"Done. Runtime preview is an LOD0 local-map built from the same chunk data the terrain uses; diff uses semantic macro/runtime ocean agreement."
	)


func _sample_macro_ocean(chunk_coord: Vector2i) -> bool:
	var key := "%d:%d" % [chunk_coord.x, chunk_coord.y]
	if _macro_ocean_cache.has(key):
		return bool(_macro_ocean_cache[key])

	var macro_map: MgBiomeMap = _generator.generate_region(
		_seed,
		float(chunk_coord.x),
		float(chunk_coord.y),
		1.0,
		1.0,
		MACRO_SAMPLE_RES,
		MACRO_SAMPLE_RES,
		0,
		1.0
	)
	var center := MACRO_SAMPLE_RES / 2
	var result := macro_map.is_ocean(center, center)
	_macro_ocean_cache[key] = result
	return result


func _build_macro_crop(n: int, img_w: int, img_h: int) -> Image:
	if _macro_texture == null:
		var blank := Image.create(img_w, img_h, false, Image.FORMAT_RGBA8)
		blank.fill(Color(0.2, 0.2, 0.2))
		return blank

	var src := _macro_texture.get_image()
	if src == null:
		var blank := Image.create(img_w, img_h, false, Image.FORMAT_RGBA8)
		blank.fill(Color(0.2, 0.2, 0.2))
		return blank

	var tex_w := src.get_width()
	var tex_h := src.get_height()
	var px_x := int(_origin.x / _macro_world_size.x * tex_w)
	var px_y := int(_origin.y / _macro_world_size.y * tex_h)
	var px_w := maxi(int(n / _macro_world_size.x * tex_w), 1)
	var px_h := maxi(int(n / _macro_world_size.y * tex_h), 1)
	px_x = clampi(px_x, 0, tex_w - 1)
	px_y = clampi(px_y, 0, tex_h - 1)
	px_w = mini(px_w, tex_w - px_x)
	px_h = mini(px_h, tex_h - px_y)

	var crop := src.get_region(Rect2i(px_x, px_y, px_w, px_h))
	crop.resize(img_w, img_h, Image.INTERPOLATE_NEAREST)
	return crop
