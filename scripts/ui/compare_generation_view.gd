extends Control

## Four-panel comparison tool.
## Top-left: macro artifact crop for world-scale context.
## Top-right: true top-down runtime local map built from LOD0 chunk data.
## Bottom-left: macro biome colours washed over runtime terrain.
## Bottom-right: semantic biome/ocean drift overlay on runtime terrain.

const CELL_PX := 128
const GRID_LINE_COLOR := Color(1.0, 1.0, 1.0, 0.16)
const GRID_LINE_STRONG := Color(0.05, 0.05, 0.06, 0.46)
const MACRO_WASH_STRENGTH := 0.34
const MATCH_OCEAN_TINT := Color(0.18, 0.42, 0.63, 1.0)
const MACRO_OCEAN_ONLY_TINT := Color(0.16, 0.86, 0.92, 1.0)
const RUNTIME_OCEAN_ONLY_TINT := Color(0.95, 0.38, 0.16, 1.0)
const WATER_BIOME_MISMATCH_TINT := Color(0.98, 0.80, 0.16, 1.0)
const WATER_BIOME_MISMATCH_ALT_TINT := Color(0.98, 0.96, 0.84, 1.0)
const LAND_BIOME_MISMATCH_TINT := Color(0.88, 0.26, 0.92, 1.0)
const RUNTIME_CHUNK_PREVIEW_RENDERER := preload("res://scripts/ui/runtime_chunk_preview_renderer.gd")

var _seed: int
var _origin: Vector2
var _grid_size: int
var _macro_texture: Texture2D
var _macro_world_size: Vector2

var _title_label: Label
var _status_label: Label
var _macro_rect: TextureRect
var _runtime_rect: TextureRect
var _wash_rect: TextureRect
var _diff_rect: TextureRect
var _agreement_label: Label
var _panels_grid: GridContainer
var _panel_rects: Array[TextureRect] = []
var _preview_renderer
var _generator := MgTerrainGen.new()
var _legend_wrap

const GRID_GAP := 8.0
const PANEL_LABEL_HEIGHT := 22.0
const TOP_RESERVE := 108.0
const BOTTOM_RESERVE := 96.0
const SINGLE_COLUMN_TOP_RESERVE := 120.0
const SINGLE_COLUMN_BOTTOM_RESERVE := 88.0

func _ready() -> void:
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	position = Vector2.ZERO
	mouse_filter = Control.MOUSE_FILTER_STOP
	resized.connect(_refresh_responsive_layout)

	_preview_renderer = RUNTIME_CHUNK_PREVIEW_RENDERER.new()
	add_child(_preview_renderer)

	var bg := ColorRect.new()
	bg.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	bg.color = Color(0.08, 0.08, 0.10, 0.96)
	add_child(bg)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", 18)
	margin.add_theme_constant_override("margin_top", 14)
	margin.add_theme_constant_override("margin_right", 18)
	margin.add_theme_constant_override("margin_bottom", 14)
	add_child(margin)

	var vbox := VBoxContainer.new()
	vbox.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	vbox.add_theme_constant_override("separation", 8)
	vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	margin.add_child(vbox)

	_title_label = Label.new()
	_title_label.text = "Compare Generation"
	_title_label.add_theme_font_size_override("font_size", 18)
	_title_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_title_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.add_child(_title_label)

	_status_label = Label.new()
	_status_label.text = "Initialising…"
	_status_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_status_label.autowrap_mode = TextServer.AUTOWRAP_WORD
	_status_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.add_child(_status_label)

	_legend_wrap = HFlowContainer.new()
	_legend_wrap.add_theme_constant_override("h_separation", 16)
	_legend_wrap.add_theme_constant_override("v_separation", 8)
	_legend_wrap.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.add_child(_legend_wrap)
	_add_legend_entry(_make_color_swatch(MATCH_OCEAN_TINT, 0.55), "matching ocean")
	_add_legend_entry(_make_color_swatch(MACRO_OCEAN_ONLY_TINT, 0.82), "macro ocean only")
	_add_legend_entry(_make_color_swatch(RUNTIME_OCEAN_ONLY_TINT, 0.82), "runtime ocean only")
	_add_legend_entry(_make_hatched_swatch(), "water-biome drift")
	_add_legend_entry(_make_color_swatch(LAND_BIOME_MISMATCH_TINT, 0.72), "land-biome drift")

	_panels_grid = GridContainer.new()
	_panels_grid.columns = 2
	_panels_grid.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_panels_grid.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_panels_grid.add_theme_constant_override("h_separation", int(GRID_GAP))
	_panels_grid.add_theme_constant_override("v_separation", int(GRID_GAP))
	vbox.add_child(_panels_grid)

	for panel_label in [
		"Macro Visual  (macro artifact)",
		"Runtime Local Map  (LOD0)",
		"Macro Colours over Runtime",
		"Delta  (Biome + Ocean Drift)",
	]:
		var col := VBoxContainer.new()
		col.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		col.size_flags_vertical = Control.SIZE_EXPAND_FILL
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
		rect.custom_minimum_size = Vector2(240.0, 240.0)
		col.add_child(rect)
		_panels_grid.add_child(col)
		_panel_rects.append(rect)
		match panel_label:
			"Macro Visual  (macro artifact)":
				_macro_rect = rect
			"Runtime Local Map  (LOD0)":
				_runtime_rect = rect
			"Macro Colours over Runtime":
				_wash_rect = rect
			"Delta  (Biome + Ocean Drift)":
				_diff_rect = rect

	_agreement_label = Label.new()
	_agreement_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_agreement_label.add_theme_font_size_override("font_size", 16)
	_agreement_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_agreement_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.add_child(_agreement_label)

	var close_btn := Button.new()
	close_btn.text = "Close"
	close_btn.pressed.connect(func(): get_parent().queue_free())
	vbox.add_child(close_btn)
	call_deferred("_refresh_responsive_layout")


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
	_draw_chunk_grid(macro_img)
	_macro_rect.texture = ImageTexture.create_from_image(macro_img)
	var macro_semantic := _build_macro_semantic_images(n, img_w, img_h)
	var macro_biome_img: Image = macro_semantic.get("biome_image")
	var macro_mask_img: Image = macro_semantic.get("ocean_mask_image")

	_status_label.text = "Building %dx%d LOD0 local-map region from runtime chunk data…" % [n, n]
	var region_preview: Dictionary = _preview_renderer.render_chunk_grid_preview(
		_seed,
		Vector2i(int(_origin.x), int(_origin.y)),
		n
	)
	var runtime_img := _copy_image(region_preview.get("image"))
	runtime_img.resize(img_w, img_h, Image.INTERPOLATE_NEAREST)
	_draw_chunk_grid(runtime_img)
	_runtime_rect.texture = ImageTexture.create_from_image(runtime_img)

	var runtime_biome_img := _copy_image(region_preview.get("biome_image"))
	runtime_biome_img.resize(img_w, img_h, Image.INTERPOLATE_NEAREST)
	var runtime_mask_img := _copy_image(region_preview.get("ocean_mask_image"))
	runtime_mask_img.resize(img_w, img_h, Image.INTERPOLATE_NEAREST)

	var wash_img := _build_macro_wash_image(runtime_img, macro_img)
	var diff_img := _build_diff_overlay_image(
		runtime_img,
		macro_biome_img,
		runtime_biome_img,
		macro_mask_img,
		runtime_mask_img
	)
	_draw_chunk_grid(wash_img)
	_draw_chunk_grid(diff_img)
	_wash_rect.texture = ImageTexture.create_from_image(wash_img)
	_diff_rect.texture = ImageTexture.create_from_image(diff_img)

	var stats := _compute_comparison_stats(macro_biome_img, runtime_biome_img, macro_mask_img, runtime_mask_img)
	var pixel_total := int(stats.get("pixel_total", 0))
	var ocean_agree := int(stats.get("ocean_agree", 0))
	var biome_agree := int(stats.get("biome_agree", 0))
	var land_biome_agree := int(stats.get("land_biome_agree", 0))
	var land_biome_total := int(stats.get("land_biome_total", 0))
	var chunk_total := int(stats.get("chunk_total", 0))
	var chunk_clean := int(stats.get("chunk_clean", 0))
	var macro_ocean_only := int(stats.get("macro_ocean_only", 0))
	var runtime_ocean_only := int(stats.get("runtime_ocean_only", 0))
	var biome_mismatch := int(stats.get("biome_mismatch", 0))
	var water_biome_mismatch := int(stats.get("water_biome_mismatch", 0))
	var land_biome_mismatch := int(stats.get("land_biome_mismatch", 0))
	var ocean_pct := 100.0 * float(ocean_agree) / float(maxi(pixel_total, 1))
	var biome_pct := 100.0 * float(biome_agree) / float(maxi(pixel_total, 1))
	var land_biome_pct := 100.0 * float(land_biome_agree) / float(maxi(land_biome_total, 1))
	_status_label.text = (
		"Done. Macro = macro artifact context. Wash = macro colours on runtime. Delta = ocean plus land and water biome drift."
	)
	_agreement_label.text = (
		"Ocean: %d/%d (%.1f%%)    Biome: %d/%d (%.1f%%)    Land biome: %d/%d (%.1f%%)\nClean chunks: %d/%d    Macro-ocean only: %d px    Runtime-ocean only: %d px    Water-biome mismatch: %d px    Land-biome mismatch: %d px    Total biome mismatch: %d px"
		% [
			ocean_agree, pixel_total, ocean_pct,
			biome_agree, pixel_total, biome_pct,
			land_biome_agree, land_biome_total, land_biome_pct,
			chunk_clean, chunk_total,
			macro_ocean_only, runtime_ocean_only,
			water_biome_mismatch, land_biome_mismatch, biome_mismatch
		]
	)


func _refresh_responsive_layout() -> void:
	var viewport_size := get_viewport_rect().size
	if viewport_size.x <= 0.0 or viewport_size.y <= 0.0 or _panels_grid == null:
		return
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	var columns := 2 if viewport_size.x >= 1100.0 else 1
	_panels_grid.columns = columns
	var rows := int(ceil(float(_panel_rects.size()) / float(columns)))
	var horizontal_padding: float = 36.0
	var available_width: float = maxf(viewport_size.x - horizontal_padding, 320.0)
	var width_limited: float = floor((available_width - GRID_GAP * float(columns - 1)) / float(columns))
	var reserve_top := TOP_RESERVE if columns == 2 else SINGLE_COLUMN_TOP_RESERVE
	var reserve_bottom := BOTTOM_RESERVE if columns == 2 else SINGLE_COLUMN_BOTTOM_RESERVE
	var available_height: float = maxf(
		viewport_size.y - reserve_top - reserve_bottom - GRID_GAP * float(rows - 1) - PANEL_LABEL_HEIGHT * float(rows),
		180.0
	)
	var height_limited: float = floor(available_height / float(rows))
	var panel_side: float = minf(width_limited, height_limited)
	panel_side = clampf(panel_side, 160.0, 440.0)
	for rect in _panel_rects:
		rect.custom_minimum_size = Vector2(panel_side, panel_side)


func _add_legend_entry(swatch: Control, text: String) -> void:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 8)
	row.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	row.alignment = BoxContainer.ALIGNMENT_CENTER
	_legend_wrap.add_child(row)
	row.add_child(swatch)

	var label := Label.new()
	label.text = text
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	row.add_child(label)


func _make_color_swatch(tint: Color, strength: float) -> Control:
	var swatch := ColorRect.new()
	swatch.custom_minimum_size = Vector2(22.0, 14.0)
	swatch.color = Color(0.32, 0.30, 0.28, 1.0).lerp(tint, strength)
	return swatch


func _make_hatched_swatch() -> Control:
	var image := Image.create(22, 14, false, Image.FORMAT_RGBA8)
	var base := Color(0.32, 0.30, 0.28, 1.0)
	for py in range(image.get_height()):
		for px in range(image.get_width()):
			var color := base
			if int((px + py) / 3) % 2 == 0:
				color = base.lerp(WATER_BIOME_MISMATCH_TINT, 0.82)
			else:
				color = base.lerp(WATER_BIOME_MISMATCH_ALT_TINT, 0.52)
			image.set_pixel(px, py, color)
	var rect := TextureRect.new()
	rect.custom_minimum_size = Vector2(22.0, 14.0)
	rect.texture = ImageTexture.create_from_image(image)
	rect.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	rect.stretch_mode = TextureRect.STRETCH_SCALE
	return rect


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


func _build_macro_semantic_images(n: int, img_w: int, img_h: int) -> Dictionary:
	var macro_map: MgBiomeMap = _generator.generate_region(
		_seed,
		_origin.x,
		_origin.y,
		float(n),
		float(n),
		img_w,
		img_h,
		0,
		1.0
	)
	var biome_rgba := macro_map.export_layer_rgba("biome")
	var biome_image := Image.create_from_data(img_w, img_h, false, Image.FORMAT_RGBA8, biome_rgba)
	var ocean_mask_image := Image.create(img_w, img_h, false, Image.FORMAT_RGBA8)
	var fluid_mask: PackedByteArray = macro_map.is_ocean_grid()
	for py in range(img_h):
		for px in range(img_w):
			var index := py * img_w + px
			var ocean := index < fluid_mask.size() and fluid_mask[index] != 0
			ocean_mask_image.set_pixel(px, py, Color.WHITE if ocean else Color.BLACK)
	return {
		"biome_image": biome_image,
		"ocean_mask_image": ocean_mask_image,
	}


func _build_macro_wash_image(runtime_img: Image, macro_img: Image) -> Image:
	var image := _copy_image(runtime_img)
	for py in range(image.get_height()):
		for px in range(image.get_width()):
			var runtime_color := image.get_pixel(px, py)
			var macro_color := macro_img.get_pixel(px, py)
			image.set_pixel(px, py, runtime_color.lerp(macro_color, MACRO_WASH_STRENGTH))
	return image


func _build_diff_overlay_image(
	runtime_img: Image,
	macro_img: Image,
	runtime_biome_img: Image,
	macro_mask_img: Image,
	runtime_mask_img: Image
) -> Image:
	var image := _copy_image(runtime_img)
	for py in range(image.get_height()):
		for px in range(image.get_width()):
			var base := image.get_pixel(px, py)
			var macro_color := macro_img.get_pixel(px, py)
			var runtime_biome_color := runtime_biome_img.get_pixel(px, py)
			var macro_ocean := _mask_pixel_is_ocean(macro_mask_img.get_pixel(px, py))
			var runtime_ocean := _mask_pixel_is_ocean(runtime_mask_img.get_pixel(px, py))
			var overlay := base
			if macro_ocean and runtime_ocean and not _colors_match(macro_color, runtime_biome_color):
				overlay = _water_biome_mismatch_overlay(base, px, py)
			elif macro_ocean and runtime_ocean:
				overlay = base.lerp(MATCH_OCEAN_TINT, 0.32)
			elif macro_ocean and not runtime_ocean:
				overlay = base.lerp(MACRO_OCEAN_ONLY_TINT, 0.82)
			elif runtime_ocean and not macro_ocean:
				overlay = base.lerp(RUNTIME_OCEAN_ONLY_TINT, 0.82)
			elif not _colors_match(macro_color, runtime_biome_color):
				overlay = base.lerp(LAND_BIOME_MISMATCH_TINT, 0.72)
			image.set_pixel(px, py, overlay)
	return image


func _water_biome_mismatch_overlay(base: Color, px: int, py: int) -> Color:
	var stripe_primary := int((px + py) / 6) % 2 == 0
	if stripe_primary:
		return base.lerp(WATER_BIOME_MISMATCH_TINT, 0.82)
	return base.lerp(WATER_BIOME_MISMATCH_ALT_TINT, 0.52)


func _compute_comparison_stats(
	macro_img: Image,
	runtime_biome_img: Image,
	macro_mask_img: Image,
	runtime_mask_img: Image
) -> Dictionary:
	var pixel_total := macro_mask_img.get_width() * macro_mask_img.get_height()
	var ocean_agree := 0
	var biome_agree := 0
	var land_biome_agree := 0
	var land_biome_total := 0
	var macro_ocean_only := 0
	var runtime_ocean_only := 0
	var biome_mismatch := 0
	var water_biome_mismatch := 0
	var land_biome_mismatch := 0
	var chunk_clean := 0
	var chunk_total := _grid_size * _grid_size
	for gy in range(_grid_size):
		for gx in range(_grid_size):
			var chunk_has_mismatch := false
			var start_x := gx * CELL_PX
			var start_y := gy * CELL_PX
			for py in range(start_y, start_y + CELL_PX):
				for px in range(start_x, start_x + CELL_PX):
					var macro_color := macro_img.get_pixel(px, py)
					var runtime_biome_color := runtime_biome_img.get_pixel(px, py)
					var macro_ocean := _mask_pixel_is_ocean(macro_mask_img.get_pixel(px, py))
					var runtime_ocean := _mask_pixel_is_ocean(runtime_mask_img.get_pixel(px, py))
					if macro_ocean == runtime_ocean:
						ocean_agree += 1
					elif macro_ocean:
						macro_ocean_only += 1
						chunk_has_mismatch = true
					else:
						runtime_ocean_only += 1
						chunk_has_mismatch = true
					if _colors_match(macro_color, runtime_biome_color):
						biome_agree += 1
						if not macro_ocean and not runtime_ocean:
							land_biome_agree += 1
					else:
						biome_mismatch += 1
						if macro_ocean or runtime_ocean:
							water_biome_mismatch += 1
						else:
							land_biome_mismatch += 1
						chunk_has_mismatch = true
					if not macro_ocean and not runtime_ocean:
						land_biome_total += 1
			if not chunk_has_mismatch:
				chunk_clean += 1
	return {
		"pixel_total": pixel_total,
		"ocean_agree": ocean_agree,
		"biome_agree": biome_agree,
		"land_biome_agree": land_biome_agree,
		"land_biome_total": land_biome_total,
		"macro_ocean_only": macro_ocean_only,
		"runtime_ocean_only": runtime_ocean_only,
		"biome_mismatch": biome_mismatch,
		"water_biome_mismatch": water_biome_mismatch,
		"land_biome_mismatch": land_biome_mismatch,
		"chunk_total": chunk_total,
		"chunk_clean": chunk_clean,
	}


func _draw_chunk_grid(image: Image) -> void:
	var w := image.get_width()
	var h := image.get_height()
	for gx in range(_grid_size + 1):
		var x := mini(gx * CELL_PX, w - 1)
		for y in range(h):
			image.set_pixel(x, y, GRID_LINE_COLOR)
			if x + 1 < w:
				image.set_pixel(x + 1, y, GRID_LINE_STRONG)
	for gy in range(_grid_size + 1):
		var y := mini(gy * CELL_PX, h - 1)
		for x in range(w):
			image.set_pixel(x, y, GRID_LINE_COLOR)
			if y + 1 < h:
				image.set_pixel(x, y + 1, GRID_LINE_STRONG)


func _copy_image(src: Image) -> Image:
	var image := Image.create(src.get_width(), src.get_height(), false, Image.FORMAT_RGBA8)
	image.blit_rect(src, Rect2i(Vector2i.ZERO, Vector2i(src.get_width(), src.get_height())), Vector2i.ZERO)
	return image


func _mask_pixel_is_ocean(color: Color) -> bool:
	return color.r > 0.5


func _colors_match(a: Color, b: Color) -> bool:
	return (
		int(round(a.r * 255.0)) == int(round(b.r * 255.0))
		and int(round(a.g * 255.0)) == int(round(b.g * 255.0))
		and int(round(a.b * 255.0)) == int(round(b.b * 255.0))
	)
