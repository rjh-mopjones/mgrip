extends Control

## Four-panel comparison tool.
## Top-left: macro biome.png crop for world-scale context.
## Top-right: true top-down runtime local map built from LOD0 chunk data.
## Bottom-left: macro biome colours washed over runtime terrain.
## Bottom-right: semantic ocean-mask drift overlay on runtime terrain.

const CELL_PX := 128
const GRID_LINE_COLOR := Color(1.0, 1.0, 1.0, 0.16)
const GRID_LINE_STRONG := Color(0.05, 0.05, 0.06, 0.46)
const MACRO_WASH_STRENGTH := 0.34
const MATCH_OCEAN_TINT := Color(0.18, 0.42, 0.63, 1.0)
const MACRO_OCEAN_ONLY_TINT := Color(0.16, 0.86, 0.92, 1.0)
const RUNTIME_OCEAN_ONLY_TINT := Color(0.95, 0.38, 0.16, 1.0)
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
var _preview_renderer


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

	var panels_grid := GridContainer.new()
	panels_grid.columns = 2
	panels_grid.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panels_grid.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panels_grid.add_theme_constant_override("h_separation", 8)
	panels_grid.add_theme_constant_override("v_separation", 8)
	vbox.add_child(panels_grid)

	for panel_label in [
		"Macro Visual  (biome.png)",
		"Runtime Local Map  (LOD0)",
		"Macro Colours over Runtime",
		"Delta  (Ocean Mask Drift)",
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
		col.add_child(rect)
		panels_grid.add_child(col)
		match panel_label:
			"Macro Visual  (biome.png)":
				_macro_rect = rect
			"Runtime Local Map  (LOD0)":
				_runtime_rect = rect
			"Macro Colours over Runtime":
				_wash_rect = rect
			"Delta  (Ocean Mask Drift)":
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
	var macro_mask_img := _build_macro_ocean_mask_image(macro_img)
	_draw_chunk_grid(macro_img)
	_macro_rect.texture = ImageTexture.create_from_image(macro_img)

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

	var runtime_mask_img := _copy_image(region_preview.get("ocean_mask_image"))
	runtime_mask_img.resize(img_w, img_h, Image.INTERPOLATE_NEAREST)

	var wash_img := _build_macro_wash_image(runtime_img, macro_img)
	var diff_img := _build_diff_overlay_image(runtime_img, macro_mask_img, runtime_mask_img)
	_draw_chunk_grid(wash_img)
	_draw_chunk_grid(diff_img)
	_wash_rect.texture = ImageTexture.create_from_image(wash_img)
	_diff_rect.texture = ImageTexture.create_from_image(diff_img)

	var stats := _compute_mask_stats(macro_mask_img, runtime_mask_img)
	var pixel_total := int(stats.get("pixel_total", 0))
	var pixel_agree := int(stats.get("pixel_agree", 0))
	var chunk_total := int(stats.get("chunk_total", 0))
	var chunk_clean := int(stats.get("chunk_clean", 0))
	var macro_ocean_only := int(stats.get("macro_ocean_only", 0))
	var runtime_ocean_only := int(stats.get("runtime_ocean_only", 0))
	var pct := 100.0 * float(pixel_agree) / float(maxi(pixel_total, 1))
	_status_label.text = (
		"Done. Left panel keeps biome.png as macro world context; bottom-left washes those biome colours over the traversed runtime terrain; bottom-right compares biome.png ocean mask vs runtime LOD0 fluid mask across the full 8x8 region."
	)
	_agreement_label.text = (
		"Pixel agreement: %d/%d (%.1f%%)    Clean chunks: %d/%d    Macro-ocean only: %d px    Runtime-ocean only: %d px"
		% [pixel_agree, pixel_total, pct, chunk_clean, chunk_total, macro_ocean_only, runtime_ocean_only]
	)

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


func _build_macro_ocean_mask_image(macro_img: Image) -> Image:
	var image := Image.create(macro_img.get_width(), macro_img.get_height(), false, Image.FORMAT_RGBA8)
	for py in range(macro_img.get_height()):
		for px in range(macro_img.get_width()):
			var color := macro_img.get_pixel(px, py)
			image.set_pixel(px, py, Color.WHITE if _pixel_is_macro_ocean(color) else Color.BLACK)
	return image


func _build_macro_wash_image(runtime_img: Image, macro_img: Image) -> Image:
	var image := _copy_image(runtime_img)
	for py in range(image.get_height()):
		for px in range(image.get_width()):
			var runtime_color := image.get_pixel(px, py)
			var macro_color := macro_img.get_pixel(px, py)
			image.set_pixel(px, py, runtime_color.lerp(macro_color, MACRO_WASH_STRENGTH))
	return image


func _build_diff_overlay_image(runtime_img: Image, macro_mask_img: Image, runtime_mask_img: Image) -> Image:
	var image := _copy_image(runtime_img)
	for py in range(image.get_height()):
		for px in range(image.get_width()):
			var base := image.get_pixel(px, py)
			var macro_ocean := _mask_pixel_is_ocean(macro_mask_img.get_pixel(px, py))
			var runtime_ocean := _mask_pixel_is_ocean(runtime_mask_img.get_pixel(px, py))
			var overlay := base
			if macro_ocean and runtime_ocean:
				overlay = base.lerp(MATCH_OCEAN_TINT, 0.32)
			elif macro_ocean and not runtime_ocean:
				overlay = base.lerp(MACRO_OCEAN_ONLY_TINT, 0.82)
			elif runtime_ocean and not macro_ocean:
				overlay = base.lerp(RUNTIME_OCEAN_ONLY_TINT, 0.82)
			image.set_pixel(px, py, overlay)
	return image


func _compute_mask_stats(macro_mask_img: Image, runtime_mask_img: Image) -> Dictionary:
	var pixel_total := macro_mask_img.get_width() * macro_mask_img.get_height()
	var pixel_agree := 0
	var macro_ocean_only := 0
	var runtime_ocean_only := 0
	var chunk_clean := 0
	var chunk_total := _grid_size * _grid_size
	for gy in range(_grid_size):
		for gx in range(_grid_size):
			var chunk_has_mismatch := false
			var start_x := gx * CELL_PX
			var start_y := gy * CELL_PX
			for py in range(start_y, start_y + CELL_PX):
				for px in range(start_x, start_x + CELL_PX):
					var macro_ocean := _mask_pixel_is_ocean(macro_mask_img.get_pixel(px, py))
					var runtime_ocean := _mask_pixel_is_ocean(runtime_mask_img.get_pixel(px, py))
					if macro_ocean == runtime_ocean:
						pixel_agree += 1
					elif macro_ocean:
						macro_ocean_only += 1
						chunk_has_mismatch = true
					else:
						runtime_ocean_only += 1
						chunk_has_mismatch = true
			if not chunk_has_mismatch:
				chunk_clean += 1
	return {
		"pixel_total": pixel_total,
		"pixel_agree": pixel_agree,
		"macro_ocean_only": macro_ocean_only,
		"runtime_ocean_only": runtime_ocean_only,
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


func _pixel_is_macro_ocean(color: Color) -> bool:
	var r := int(round(color.r * 255.0))
	var g := int(round(color.g * 255.0))
	var b := int(round(color.b * 255.0))
	var rf := float(r) / 255.0
	var bf := float(b) / 255.0
	if bf - rf > 0.25 and bf > 0.35:
		return true
	return (r == 200 and g == 100 and b == 120) or (r == 120 and g == 80 and b == 60)
