extends Control

## Three-panel comparison tool.
## Left:  biome.png crop — what the world map shows for this region.
## Middle: per-chunk runtime noise at freq=8.0 — what actually generates in-game.
## Right:  diff — green where biome.png and runtime agree on ocean/land, red where they don't.

const CELL_PX        := 64
const MICRO_RES      := 65
const AGREE_COLOR    := Color(0.12, 0.78, 0.31, 1.0)
const DISAGREE_COLOR := Color(0.82, 0.18, 0.18, 1.0)

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


func _ready() -> void:
	var vp_size := get_viewport().get_visible_rect().size
	position = Vector2.ZERO
	size = vp_size
	mouse_filter = Control.MOUSE_FILTER_STOP

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

	for panel_label in ["Macro  (biome.png)", "Micro  (freq=8.0  runtime)", "Diff"]:
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
			"Macro  (biome.png)":          _macro_rect = rect
			"Micro  (freq=8.0  runtime)":  _micro_rect = rect
			"Diff":                         _diff_rect  = rect

	_agreement_label = Label.new()
	_agreement_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_agreement_label.add_theme_font_size_override("font_size", 16)
	vbox.add_child(_agreement_label)

	var close_btn := Button.new()
	close_btn.text = "Close"
	close_btn.pressed.connect(func(): get_parent().queue_free())
	vbox.add_child(close_btn)


func show_comparison(
	seed: int, wx: float, wy: float, grid_size: int,
	macro_tex: Texture2D, macro_world: Vector2
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
		"Generating — meso (%d,%d)  seed=%d  world (%d,%d)…"
		% [mx, my, seed, int(wx), int(wy)]
	)
	_agreement_label.text = ""
	call_deferred("_generate")


func _generate() -> void:
	var n     := _grid_size
	var img_w := n * CELL_PX
	var img_h := n * CELL_PX

	# ── Macro: biome.png crop ─────────────────────────────────────────────────────
	var macro_img := _build_macro_crop(n, img_w, img_h)
	_macro_rect.texture = ImageTexture.create_from_image(macro_img)

	# ── Micro grid + diff ─────────────────────────────────────────────────────────
	var micro_img := Image.create(img_w, img_h, false, Image.FORMAT_RGBA8)
	var diff_img  := Image.create(img_w, img_h, false, Image.FORMAT_RGBA8)
	var gen := MgTerrainGen.new()
	var agree_count := 0
	var total := n * n

	for gy in range(n):
		for gx in range(n):
			var cx: float = _origin.x + gx
			var cy: float = _origin.y + gy

			# Macro ocean: sample centre of this chunk cell in the biome.png crop
			var macro_ocean := _pixel_is_ocean(
				macro_img.get_pixel(gx * CELL_PX + CELL_PX / 2, gy * CELL_PX + CELL_PX / 2)
			)

			# Micro runtime: LOD2 chunk at freq=8.0
			var micro_map: MgBiomeMap = gen.generate_chunk_lod(_seed, cx, cy, MICRO_RES, 0, 8.0)
			var mc := MICRO_RES / 2
			var micro_ocean: bool = micro_map.is_ocean(mc, mc)

			var agree: bool = macro_ocean == micro_ocean
			if agree:
				agree_count += 1

			# Blit micro biome texture (MICRO_RES → CELL_PX, nearest-neighbour)
			var micro_rgba := micro_map.export_layer_rgba("biome")
			var micro_cell := Image.create_from_data(MICRO_RES, MICRO_RES, false, Image.FORMAT_RGBA8, micro_rgba)
			micro_cell.resize(CELL_PX, CELL_PX, Image.INTERPOLATE_NEAREST)
			micro_img.blit_rect(micro_cell, Rect2i(0, 0, CELL_PX, CELL_PX), Vector2i(gx * CELL_PX, gy * CELL_PX))

			diff_img.fill_rect(
				Rect2i(gx * CELL_PX, gy * CELL_PX, CELL_PX, CELL_PX),
				AGREE_COLOR if agree else DISAGREE_COLOR
			)

	_micro_rect.texture = ImageTexture.create_from_image(micro_img)
	_diff_rect.texture  = ImageTexture.create_from_image(diff_img)

	var pct := 100.0 * agree_count / total
	_agreement_label.text = "Agreement: %d/%d  (%.1f%%)" % [agree_count, total, pct]
	_status_label.text = "Done. Red cells = biome.png shows ocean but runtime generates land (erosion gap)."


## Ocean biomes as defined by tile_has_fluid_surface() in Rust:
##   Sea[0,191,255], ShallowSea[100,200,240], ContinentalShelf[70,150,200],
##   DeepOcean[0,40,100], OceanTrench[0,51,102]  — blue-dominant, caught by heuristic
##   OceanRidge[120,80,60]                        — brownish, NOT blue-dominant
##   CoralReef[200,100,120]                       — pinkish, NOT blue-dominant
## biome.png stores exact integer biome colors (nearest-neighbour, no blending), so we can
## match non-blue ocean biomes with exact 8-bit integer comparisons.
func _pixel_is_ocean(color: Color) -> bool:
	if color.b - color.r > 0.25 and color.b > 0.35:
		return true
	var ri := roundi(color.r * 255)
	var gi := roundi(color.g * 255)
	var bi := roundi(color.b * 255)
	# CoralReef: rgb(200, 100, 120)
	if ri == 200 and gi == 100 and bi == 120:
		return true
	# OceanRidge: rgb(120, 80, 60)  — Scrubland(115,80,65) is 5 units away; exact match is safe
	if ri == 120 and gi == 80 and bi == 60:
		return true
	return false


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
