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
	var image: Image = _build_chunk_map_image(heights, fluid_mask, biome_rgba, texture_size)
	var ocean_mask_image: Image = _build_ocean_mask_image(fluid_mask, texture_size)
	var texture := ImageTexture.create_from_image(image)
	var center := CHUNK_RESOLUTION / 2
	return {
		"image": image,
		"ocean_mask_image": ocean_mask_image,
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
	var ocean_mask_image := Image.create(
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
			var chunk_ocean_mask: Image = preview.get("ocean_mask_image")
			image.blit_rect(
				chunk_image,
				Rect2i(Vector2i.ZERO, Vector2i(cell_size, cell_size)),
				Vector2i(gx * cell_size, gy * cell_size)
			)
			ocean_mask_image.blit_rect(
				chunk_ocean_mask,
				Rect2i(Vector2i.ZERO, Vector2i(cell_size, cell_size)),
				Vector2i(gx * cell_size, gy * cell_size)
			)
			cell_ocean["%d:%d" % [chunk_coord.x, chunk_coord.y]] = bool(preview.get("micro_ocean", false))

	var texture := ImageTexture.create_from_image(image)
	var result := {
		"image": image,
		"ocean_mask_image": ocean_mask_image,
		"texture": texture,
		"cell_ocean": cell_ocean,
	}
	_cache[key] = result
	return result


func _build_chunk_map_image(
	heights: PackedInt32Array,
	fluid_mask: PackedByteArray,
	biome_rgba: PackedByteArray,
	texture_size: int
) -> Image:
	var image := Image.create(texture_size, texture_size, false, Image.FORMAT_RGBA8)
	for py in range(texture_size):
		var sy := _sample_coord(py, texture_size)
		for px in range(texture_size):
			var sx := _sample_coord(px, texture_size)
			var index := sy * CHUNK_RESOLUTION + sx
			var color := _sample_topdown_color(heights, fluid_mask, biome_rgba, sx, sy, index)
			image.set_pixel(px, py, color)
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
