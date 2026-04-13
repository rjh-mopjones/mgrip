extends Node
class_name RuntimeChunkPreviewRenderer

const DEFAULT_PREVIEW_SIZE := 256
const DEFAULT_COMPARE_CELL_SIZE := 128
const CHUNK_RESOLUTION := GenerationManager.LOD0_RESOLUTION
const HEIGHT_SCALE := VoxelMeshBuilder.HEIGHT_SCALE
const SEA_LEVEL_Y := VoxelMeshBuilder.SEA_LEVEL_Y

const LAND_LOW := Color(0.54, 0.45, 0.36, 1.0)
const LAND_HIGH := Color(0.89, 0.84, 0.76, 1.0)
const CLIFF := Color(0.30, 0.26, 0.26, 1.0)
const CONTOUR := Color(0.18, 0.14, 0.12, 1.0)
const WATER_SHALLOW := Color(0.22, 0.50, 0.76, 1.0)
const WATER_DEEP := Color(0.05, 0.18, 0.40, 1.0)
const BIOME_TINT_STRENGTH := 0.42

var _cache: Dictionary = {}


func clear_cache() -> void:
	_cache.clear()


func render_chunk_preview(
	seed: int,
	chunk_coord: Vector2i,
	use_cache: bool = true,
	texture_size: int = DEFAULT_PREVIEW_SIZE
) -> Dictionary:
	var key := "chunk:%d:%d:%d:%d" % [seed, chunk_coord.x, chunk_coord.y, texture_size]
	if use_cache and _cache.has(key):
		return _cache[key]

	var biome_map := GenerationManager.generate_runtime_chunk_for_lod_with_seed(
		seed,
		chunk_coord,
		GenerationManager.LOD0_NAME,
	)
	var result := render_biome_map_preview(biome_map, texture_size)
	if use_cache:
		_cache[key] = result
	return result


func render_biome_map_preview(
	biome_map: MgBiomeMap,
	texture_size: int = DEFAULT_PREVIEW_SIZE
) -> Dictionary:
	var summary: Dictionary = biome_map.build_runtime_chunk_presentation_data().get("summary", {}).duplicate(true)
	var heights: PackedInt32Array = biome_map.block_heights(HEIGHT_SCALE)
	var fluid_mask: PackedByteArray = biome_map.is_ocean_grid()
	var biome_rgba: PackedByteArray = biome_map.export_layer_rgba("biome")
	var rivers: PackedByteArray = biome_map.rivers_byte_grid()
	var aridity: PackedByteArray = biome_map.aridity_byte_grid()
	var temperature: PackedByteArray = biome_map.temperature_byte_grid()
	var light_level: PackedByteArray = biome_map.light_level_byte_grid()
	var image: Image = _build_chunk_map_image(
		heights, fluid_mask, biome_rgba, rivers, aridity, temperature, light_level, texture_size
	)
	var biome_image: Image = _build_biome_image(biome_rgba, texture_size)
	var ocean_mask_image: Image = _build_ocean_mask_image(fluid_mask, texture_size)
	var rivers_image: Image = _build_rivers_image(rivers, texture_size)
	var texture := ImageTexture.create_from_image(image)
	var center := CHUNK_RESOLUTION / 2
	return {
		"image": image,
		"biome_image": biome_image,
		"ocean_mask_image": ocean_mask_image,
		"rivers_image": rivers_image,
		"texture": texture,
		"micro_ocean": biome_map.is_ocean(center, center),
		"summary": summary,
	}


func render_chunk_grid_preview(
	seed: int,
	origin_chunk: Vector2i,
	grid_size: int,
	cell_size: int = DEFAULT_COMPARE_CELL_SIZE
) -> Dictionary:
	var key := "grid:%d:%d:%d:%d:%d" % [seed, origin_chunk.x, origin_chunk.y, grid_size, cell_size]
	if _cache.has(key):
		return _cache[key]

	var image := Image.create(cell_size * grid_size, cell_size * grid_size, false, Image.FORMAT_RGBA8)
	var biome_image := Image.create(cell_size * grid_size, cell_size * grid_size, false, Image.FORMAT_RGBA8)
	var ocean_mask_image := Image.create(
		cell_size * grid_size,
		cell_size * grid_size,
		false,
		Image.FORMAT_RGBA8
	)
	var rivers_image := Image.create(
		cell_size * grid_size,
		cell_size * grid_size,
		false,
		Image.FORMAT_RGBA8
	)
	var cell_ocean := {}
	for gy in range(grid_size):
		for gx in range(grid_size):
			var chunk_coord := origin_chunk + Vector2i(gx, gy)
			var preview: Dictionary = render_chunk_preview(seed, chunk_coord, true, cell_size)
			var chunk_image: Image = preview.get("image")
			var chunk_biome_image: Image = preview.get("biome_image")
			var chunk_ocean_mask: Image = preview.get("ocean_mask_image")
			var chunk_rivers: Image = preview.get("rivers_image")
			image.blit_rect(
				chunk_image,
				Rect2i(Vector2i.ZERO, Vector2i(cell_size, cell_size)),
				Vector2i(gx * cell_size, gy * cell_size)
			)
			biome_image.blit_rect(
				chunk_biome_image,
				Rect2i(Vector2i.ZERO, Vector2i(cell_size, cell_size)),
				Vector2i(gx * cell_size, gy * cell_size)
			)
			ocean_mask_image.blit_rect(
				chunk_ocean_mask,
				Rect2i(Vector2i.ZERO, Vector2i(cell_size, cell_size)),
				Vector2i(gx * cell_size, gy * cell_size)
			)
			if chunk_rivers != null:
				rivers_image.blit_rect(
					chunk_rivers,
					Rect2i(Vector2i.ZERO, Vector2i(cell_size, cell_size)),
					Vector2i(gx * cell_size, gy * cell_size)
				)
			cell_ocean["%d:%d" % [chunk_coord.x, chunk_coord.y]] = bool(preview.get("micro_ocean", false))

	var texture := ImageTexture.create_from_image(image)
	var result := {
		"image": image,
		"biome_image": biome_image,
		"ocean_mask_image": ocean_mask_image,
		"rivers_image": rivers_image,
		"texture": texture,
		"cell_ocean": cell_ocean,
	}
	_cache[key] = result
	return result


func _build_chunk_map_image(
	heights: PackedInt32Array,
	fluid_mask: PackedByteArray,
	biome_rgba: PackedByteArray,
	rivers: PackedByteArray,
	aridity: PackedByteArray,
	temperature: PackedByteArray,
	light_level: PackedByteArray,
	texture_size: int
) -> Image:
	var image := Image.create(texture_size, texture_size, false, Image.FORMAT_RGBA8)
	# Source block step — chunk is CHUNK_RESOLUTION (512), texture is `texture_size`.
	# Rivers in `rivers` are 1-3 px wide at chunk resolution. Sampling one nearest
	# pixel per texture pixel (the previous behaviour) aliased the whole channel
	# away on every chunk. We MAX over the source block so any river pixel inside
	# the block surfaces in the preview.
	var step: int = maxi(int(CHUNK_RESOLUTION / texture_size), 1)
	for py in range(texture_size):
		var sy := _sample_coord(py, texture_size)
		for px in range(texture_size):
			var sx := _sample_coord(px, texture_size)
			var index := sy * CHUNK_RESOLUTION + sx
			var color := _sample_topdown_color(heights, fluid_mask, biome_rgba, sx, sy, index)
			# Scan an `step × step` source block centred on the sampled pixel for
			# the strongest river value. Takes the river color from that peak
			# sample so we get climate-correct water tinting.
			var peak_river := 0.0
			var peak_index := index
			var by_start := maxi(sy - step / 2, 0)
			var by_end := mini(by_start + step, CHUNK_RESOLUTION)
			var bx_start := maxi(sx - step / 2, 0)
			var bx_end := mini(bx_start + step, CHUNK_RESOLUTION)
			for by in range(by_start, by_end):
				for bx in range(bx_start, bx_end):
					var bi := by * CHUNK_RESOLUTION + bx
					if bi >= rivers.size():
						continue
					var rv := float(rivers[bi]) / 255.0
					if rv > peak_river:
						peak_river = rv
						peak_index = bi
			# Per CLAUDE.md: no rivers in ocean cells. River stops at coastline.
			if peak_river > 0.005:
				var is_ocean_cell := peak_index < fluid_mask.size() and fluid_mask[peak_index] != 0
				if not is_ocean_cell:
					var arid := 0.0 if peak_index >= aridity.size() else float(aridity[peak_index]) / 255.0
					var temp := 0.0 if peak_index >= temperature.size() else float(temperature[peak_index]) / 255.0 * 150.0 - 50.0
					var light := 0.0 if peak_index >= light_level.size() else float(light_level[peak_index]) / 255.0
					var river_color := _solid_river_color(temp, light, arid)
					if peak_river >= 0.05:
						color = river_color
					else:
						var t := clampf(peak_river / 0.05, 0.0, 1.0)
						color = color.lerp(river_color, t)
			image.set_pixel(px, py, color)
	return image


func _solid_river_color(temperature: float, light: float, _aridity: float) -> Color:
	if temperature < -1.0 or light < 0.12:
		return Color(160.0 / 255.0, 190.0 / 255.0, 210.0 / 255.0, 1.0)
	return Color(80.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0)


func _build_biome_image(biome_rgba: PackedByteArray, texture_size: int) -> Image:
	var image := Image.create(texture_size, texture_size, false, Image.FORMAT_RGBA8)
	for py in range(texture_size):
		var sy := _sample_coord(py, texture_size)
		for px in range(texture_size):
			var sx := _sample_coord(px, texture_size)
			var index := sy * CHUNK_RESOLUTION + sx
			image.set_pixel(px, py, _biome_color_at(biome_rgba, index))
	return image


func _build_rivers_image(rivers: PackedByteArray, texture_size: int) -> Image:
	# Single-channel river presence as luminance — used by compare engine to
	# detect runtime rivers without going through the rendered preview pixels.
	# MAX over the source block, mirroring _build_chunk_map_image so the
	# compare metric reads the same data the visual preview shows.
	var image := Image.create(texture_size, texture_size, false, Image.FORMAT_RGBA8)
	var step: int = maxi(int(CHUNK_RESOLUTION / texture_size), 1)
	for py in range(texture_size):
		var sy := _sample_coord(py, texture_size)
		for px in range(texture_size):
			var sx := _sample_coord(px, texture_size)
			var by_start := maxi(sy - step / 2, 0)
			var by_end := mini(by_start + step, CHUNK_RESOLUTION)
			var bx_start := maxi(sx - step / 2, 0)
			var bx_end := mini(bx_start + step, CHUNK_RESOLUTION)
			var peak := 0
			for by in range(by_start, by_end):
				for bx in range(bx_start, bx_end):
					var bi := by * CHUNK_RESOLUTION + bx
					if bi >= rivers.size():
						continue
					var v := int(rivers[bi])
					if v > peak:
						peak = v
			var l := float(peak) / 255.0
			image.set_pixel(px, py, Color(l, l, l, 1.0))
	return image


func _build_ocean_mask_image(fluid_mask: PackedByteArray, texture_size: int) -> Image:
	var image := Image.create(texture_size, texture_size, false, Image.FORMAT_RGBA8)
	for py in range(texture_size):
		var sy := _sample_coord(py, texture_size)
		for px in range(texture_size):
			var sx := _sample_coord(px, texture_size)
			var index := sy * CHUNK_RESOLUTION + sx
			var ocean := index < fluid_mask.size() and fluid_mask[index] != 0
			image.set_pixel(px, py, Color.WHITE if ocean else Color.BLACK)
	return image


func _sample_coord(pixel: int, texture_size: int) -> int:
	if texture_size <= 1:
		return 0
	return mini(int(round(float(pixel) / float(texture_size - 1) * float(CHUNK_RESOLUTION - 1))), CHUNK_RESOLUTION - 1)


func _sample_topdown_color(
	heights: PackedInt32Array,
	fluid_mask: PackedByteArray,
	biome_rgba: PackedByteArray,
	sx: int,
	sy: int,
	index: int
) -> Color:
	var h := heights[index]
	var ocean := index < fluid_mask.size() and fluid_mask[index] != 0
	var left := heights[sy * CHUNK_RESOLUTION + maxi(sx - 1, 0)]
	var right := heights[sy * CHUNK_RESOLUTION + mini(sx + 1, CHUNK_RESOLUTION - 1)]
	var up := heights[maxi(sy - 1, 0) * CHUNK_RESOLUTION + sx]
	var down := heights[mini(sy + 1, CHUNK_RESOLUTION - 1) * CHUNK_RESOLUTION + sx]

	var dx := float(right - left)
	var dz := float(down - up)
	var normal := Vector3(-dx, 18.0, -dz).normalized()
	var light_dir := Vector3(-0.55, 0.75, -0.35).normalized()
	var hillshade := clampf(normal.dot(light_dir) * 0.5 + 0.5, 0.0, 1.0)
	var slope := clampf((absf(dx) + absf(dz)) / 20.0, 0.0, 1.0)

	if ocean:
		var depth_t := clampf(float(SEA_LEVEL_Y - h) / 18.0, 0.0, 1.0)
		var water := WATER_SHALLOW.lerp(WATER_DEEP, depth_t)
		var coast_mix := clampf(slope * 1.6, 0.0, 1.0)
		water = water.lerp(Color(0.64, 0.78, 0.88, 1.0), coast_mix * 0.22)
		return water.darkened((1.0 - hillshade) * 0.18)

	var height_t := clampf((float(h) + 24.0) / 120.0, 0.0, 1.0)
	var land := LAND_LOW.lerp(LAND_HIGH, height_t)
	land = land.lerp(CLIFF, slope * 0.45)
	var contour_phase := absf(fposmod(float(h), 10.0) - 5.0)
	if contour_phase < 0.75:
		land = land.lerp(CONTOUR, 0.28)
	var shade := lerpf(0.72, 1.08, hillshade)
	var shaded_land := land * shade
	var biome_color := _biome_color_at(biome_rgba, index)
	return shaded_land.lerp(biome_color, BIOME_TINT_STRENGTH)


func _biome_color_at(biome_rgba: PackedByteArray, index: int) -> Color:
	var offset := index * 4
	if offset + 3 >= biome_rgba.size():
		return LAND_LOW
	return Color(
		float(biome_rgba[offset]) / 255.0,
		float(biome_rgba[offset + 1]) / 255.0,
		float(biome_rgba[offset + 2]) / 255.0,
		1.0
	)
