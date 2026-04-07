extends RefCounted
class_name VoxelMeshBuilder

## Builds Minecraft-style voxel terrain meshes from a BiomeMap chunk.
##
## Land terrain: smooth heightfield surface derived from block heights.
## Ocean columns: no solid geometry — a separate flat water quad at SEA_LEVEL_Y.
## Split into 64 sub-meshes (8×8 of 64×64 blocks) to keep draw calls sane.

const CHUNK_SIZE   := 512
const HEIGHT_SCALE := 200.0
const SUB_SIZE     := 64
const GRID_COUNT: int = CHUNK_SIZE / SUB_SIZE  # 8

## Sea-level in block-space: floor(-0.01 * 200) = -2
const SEA_LEVEL_Y  := -2

## Geological substrate color blended into cliff faces as depth increases.
const SUBSTRATE := Color(0.46, 0.34, 0.27)
const HAZE_TINT := Color(0.75, 0.54, 0.37)
const RIDGE_TINT := Color(0.63, 0.46, 0.39)

## Biome index → Color (matches mg_core TileType enum, Sea=0 … ScorchedRock=48).
## Palette stays in blue-violet, burgundy, umber, and ash tones only.
const BIOME_COLORS: Array[Color] = [
	Color8( 38,  70, 145),  #  0 Sea
	Color8( 76, 108, 176),  #  1 ShallowSea
	Color8( 60,  90, 148),  #  2 ContinentalShelf
	Color8( 16,  28,  78),  #  3 DeepOcean
	Color8( 12,  18,  54),  #  4 OceanTrench
	Color8(120,  80,  60),  #  5 OceanRidge
	Color8( 78,  96, 176),  #  6 River
	Color8(220, 182, 130),  #  7 Beach
	Color8( 38,  18,  32),  #  8 Mangrove
	Color8(200, 100, 120),  #  9 CoralReef
	Color8( 98,  95, 108),  # 10 RockyCoast
	Color8(135, 135, 155),  # 11 SeaCliff
	Color8(250, 252, 255),  # 12 White
	Color8(214, 220, 255),  # 13 Glacier
	Color8(238, 242, 255),  # 14 Snow
	Color8(224, 230, 255),  # 15 IceSheet
	Color8( 85,  70, 110),  # 16 FrozenBog
	Color8(110,  90, 130),  # 17 Tundra
	Color8( 55,  40,  70),  # 18 Taiga
	Color8( 95,  72, 108),  # 19 AlpineMeadow
	Color8(110,  65,  88),  # 20 Plains
	Color8( 95,  60,  80),  # 21 Meadow
	Color8( 55,  30,  48),  # 22 Forest
	Color8( 75,  42,  62),  # 23 DeciduousForest
	Color8( 45,  22,  40),  # 24 TemperateRainforest
	Color8( 88,  48,  68),  # 25 Woodland
	Color8(115,  80,  65),  # 26 Scrubland
	Color8( 45,  30,  55),  # 27 Marsh
	Color8(145, 115,  85),  # 28 Steppe
	Color8(105, 105, 112),  # 29 Mountain
	Color8(130,  75,  55),  # 30 Plateau
	Color8( 40,  15,  30),  # 31 SubtropicalForest
	Color8(130,  95,  65),  # 32 DryWoodland
	Color8(140, 100,  65),  # 33 Thornland
	Color8(170, 145,  90),  # 34 HighlandSavanna
	Color8( 22,   8,  28),  # 35 CloudForest
	Color8(185, 162,  95),  # 36 Savanna
	Color8( 22,   5,  18),  # 37 Jungle
	Color8(255, 210,  90),  # 38 Desert
	Color8(248, 168,  60),  # 39 Sahara
	Color8(232, 205, 130),  # 40 Erg
	Color8(130,  92,  68),  # 41 Hamada
	Color8(238, 232, 215),  # 42 SaltFlat
	Color8(175,  98,  62),  # 43 Badlands
	Color8( 55,  18,  45),  # 44 Oasis
	Color8( 64,  28,  28),  # 45 Volcanic
	Color8( 90,  35,  20),  # 46 LavaField
	Color8(110,  25,  10),  # 47 MoltenWaste
	Color8( 58,  52,  48),  # 48 ScorchedRock
]

## Build terrain + water meshes and attach to parent.
## Returns heights and ocean_mask so world.gd can use them for spawn + collision.
func build_terrain(biome_map: MgBiomeMap, parent: Node3D) -> Dictionary:
	var heights    := biome_map.block_heights(HEIGHT_SCALE)
	var biome_rgba := biome_map.export_layer_rgba("biome")
	var ocean_mask := biome_map.is_ocean_grid()

	var land_mat := ShaderMaterial.new()
	land_mat.shader = preload("res://assets/shaders/terrain.gdshader")

	var water_mat := ShaderMaterial.new()
	water_mat.shader = preload("res://assets/shaders/water.gdshader")

	for gz in GRID_COUNT:
		for gx in GRID_COUNT:
			var ox := gx * SUB_SIZE
			var oz := gz * SUB_SIZE

			# Land terrain
			var land_mesh := _build_land_sub(heights, biome_rgba, ocean_mask, ox, oz)
			if land_mesh:
				var mi := MeshInstance3D.new()
				mi.mesh = land_mesh
				mi.material_override = land_mat
				parent.add_child(mi)

			# Water surface
			var water_mesh := _build_water_sub(ocean_mask, ox, oz)
			if water_mesh:
				var mi := MeshInstance3D.new()
				mi.mesh = water_mesh
				mi.material_override = water_mat
				parent.add_child(mi)

	return {"heights": heights, "ocean_mask": ocean_mask}

# ── Land sub-mesh ─────────────────────────────────────────────────────────────

func _build_land_sub(
		heights: PackedInt32Array,
		biome_rgba: PackedByteArray,
		ocean_mask: PackedByteArray,
		ox: int, oz: int) -> ArrayMesh:
	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	var has_geo := false

	var max_x := mini(ox + SUB_SIZE - 1, CHUNK_SIZE - 2)
	var max_z := mini(oz + SUB_SIZE - 1, CHUNK_SIZE - 2)

	for z in range(oz, max_z + 1):
		for x in range(ox, max_x + 1):
			if _cell_is_ocean(ocean_mask, x, z):
				continue
			_emit_terrain_triangle(st, heights, biome_rgba, x, z, x + 1, z, x, z + 1)
			_emit_terrain_triangle(st, heights, biome_rgba, x + 1, z, x + 1, z + 1, x, z + 1)
			has_geo = true

	if not has_geo:
		return null
	return st.commit()

# ── Water sub-mesh ────────────────────────────────────────────────────────────

func _build_water_sub(ocean_mask: PackedByteArray, ox: int, oz: int) -> ArrayMesh:
	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	var has_geo := false
	var yf := float(SEA_LEVEL_Y)

	for lz in SUB_SIZE:
		for lx in SUB_SIZE:
			var x   := ox + lx
			var z   := oz + lz
			var idx := z * CHUNK_SIZE + x
			if not ocean_mask[idx]:
				continue
			var x0 := float(x)
			var x1 := x0 + 1.0
			var z0 := float(z)
			var z1 := z0 + 1.0
			st.set_normal(Vector3.UP)
			st.add_vertex(Vector3(x0, yf, z0))
			st.add_vertex(Vector3(x1, yf, z0))
			st.add_vertex(Vector3(x1, yf, z1))
			st.add_vertex(Vector3(x0, yf, z0))
			st.add_vertex(Vector3(x1, yf, z1))
			st.add_vertex(Vector3(x0, yf, z1))
			has_geo = true

	if not has_geo:
		return null
	return st.commit()

# ── Terrain surface helpers ───────────────────────────────────────────────────

func _cell_is_ocean(ocean_mask: PackedByteArray, x: int, z: int) -> bool:
	var i00 := z * CHUNK_SIZE + x
	var i10 := z * CHUNK_SIZE + (x + 1)
	var i01 := (z + 1) * CHUNK_SIZE + x
	var i11 := (z + 1) * CHUNK_SIZE + (x + 1)
	return ocean_mask[i00] and ocean_mask[i10] and ocean_mask[i01] and ocean_mask[i11]

func _emit_terrain_triangle(
		st: SurfaceTool,
		heights: PackedInt32Array,
		biome_rgba: PackedByteArray,
		x0: int, z0: int,
		x1: int, z1: int,
		x2: int, z2: int) -> void:
	_emit_terrain_vertex(st, heights, biome_rgba, x0, z0)
	_emit_terrain_vertex(st, heights, biome_rgba, x1, z1)
	_emit_terrain_vertex(st, heights, biome_rgba, x2, z2)

func _emit_terrain_vertex(
		st: SurfaceTool,
		heights: PackedInt32Array,
		biome_rgba: PackedByteArray,
		x: int, z: int) -> void:
	var y := _heightf(heights, x, z)
	st.set_color(_vertex_color(heights, biome_rgba, x, z))
	st.set_normal(_surface_normal(heights, x, z))
	st.add_vertex(Vector3(float(x), y, float(z)))

# ── Face helpers ──────────────────────────────────────────────────────────────

func _top_face(st: SurfaceTool, x: int, y: int, z: int, col: Color) -> void:
	var x0 := float(x);      var x1 := x0 + 1.0
	var yf := float(y) + 1.0
	var z0 := float(z);      var z1 := z0 + 1.0
	st.set_color(col)
	st.set_normal(Vector3.UP)
	st.add_vertex(Vector3(x0, yf, z0))
	st.add_vertex(Vector3(x1, yf, z0))
	st.add_vertex(Vector3(x1, yf, z1))
	st.add_vertex(Vector3(x0, yf, z0))
	st.add_vertex(Vector3(x1, yf, z1))
	st.add_vertex(Vector3(x0, yf, z1))

func _surface_color(base_col: Color, y: int) -> Color:
	var ridge := clampf((float(y) - 18.0) / 64.0, 0.0, 1.0)
	var haze := clampf((float(y) + 8.0) / 96.0, 0.0, 1.0)
	var col := base_col.lerp(RIDGE_TINT, ridge * 0.10)
	col = col.lerp(HAZE_TINT, 0.04 + haze * 0.06)
	return Color(
		clampf(col.r * 0.97, 0.0, 1.0),
		clampf(col.g * 0.95, 0.0, 1.0),
		clampf(col.b * 0.94, 0.0, 1.0),
		1.0
	)

func _cliff_color(surface_col: Color, exposure: float) -> Color:
	var depth := clampf((exposure - 1.0) / 6.0, 0.0, 1.0)
	var col := surface_col.lerp(SUBSTRATE, 0.26 + depth * 0.34)
	col = col.darkened(0.10 + depth * 0.10)
	return col.lerp(HAZE_TINT, 0.04)

func _vertex_color(
		heights: PackedInt32Array,
		biome_rgba: PackedByteArray,
		x: int, z: int) -> Color:
	var idx := z * CHUNK_SIZE + x
	var bi := idx * 4
	var base := Color(
		biome_rgba[bi] / 255.0,
		biome_rgba[bi + 1] / 255.0,
		biome_rgba[bi + 2] / 255.0,
		1.0
	)
	return _surface_color(base, heights[idx])

func _surface_normal(heights: PackedInt32Array, x: int, z: int) -> Vector3:
	var left := _heightf(heights, maxi(x - 1, 0), z)
	var right := _heightf(heights, mini(x + 1, CHUNK_SIZE - 1), z)
	var back := _heightf(heights, x, maxi(z - 1, 0))
	var forward := _heightf(heights, x, mini(z + 1, CHUNK_SIZE - 1))
	return Vector3(left - right, 2.0, back - forward).normalized()

func _heightf(heights: PackedInt32Array, x: int, z: int) -> float:
	var sum := 0.0
	var weight_sum := 0.0
	for dz in range(-1, 2):
		for dx in range(-1, 2):
			var sx := clampi(x + dx, 0, CHUNK_SIZE - 1)
			var sz := clampi(z + dz, 0, CHUNK_SIZE - 1)
			var weight := 2.0 if dx == 0 and dz == 0 else 1.0
			sum += (float(heights[sz * CHUNK_SIZE + sx]) + 1.0) * weight
			weight_sum += weight
	return sum / weight_sum

## Emit a single (possibly multi-block-wide) side quad for one direction.
## `run` extends along z for dx faces, along x for dz faces.
func _emit_side_face(
		st: SurfaceTool,
		x: int, y: int, ny: int, z: int,
		dx: int, dz: int,
		run: int,
		col: Color) -> void:
	var x0    := float(x)
	var z0    := float(z)
	var y_top := float(y)  + 1.0
	var y_bot := float(ny) + 1.0
	st.set_color(col)
	if dx == 1:
		var x1 := x0 + 1.0
		var zr := z0 + float(run)
		st.set_normal(Vector3.RIGHT)
		st.add_vertex(Vector3(x1, y_top, zr))
		st.add_vertex(Vector3(x1, y_bot, zr))
		st.add_vertex(Vector3(x1, y_bot, z0))
		st.add_vertex(Vector3(x1, y_top, zr))
		st.add_vertex(Vector3(x1, y_bot, z0))
		st.add_vertex(Vector3(x1, y_top, z0))
	elif dx == -1:
		var zr := z0 + float(run)
		st.set_normal(-Vector3.RIGHT)
		st.add_vertex(Vector3(x0, y_top, z0))
		st.add_vertex(Vector3(x0, y_bot, z0))
		st.add_vertex(Vector3(x0, y_bot, zr))
		st.add_vertex(Vector3(x0, y_top, z0))
		st.add_vertex(Vector3(x0, y_bot, zr))
		st.add_vertex(Vector3(x0, y_top, zr))
	elif dz == 1:
		var z1 := z0 + 1.0
		var xr := x0 + float(run)
		st.set_normal(Vector3.BACK)
		st.add_vertex(Vector3(x0, y_top, z1))
		st.add_vertex(Vector3(x0, y_bot, z1))
		st.add_vertex(Vector3(xr, y_bot, z1))
		st.add_vertex(Vector3(x0, y_top, z1))
		st.add_vertex(Vector3(xr, y_bot, z1))
		st.add_vertex(Vector3(xr, y_top, z1))
	else:  # dz == -1
		var xr := x0 + float(run)
		st.set_normal(-Vector3.BACK)
		st.add_vertex(Vector3(xr, y_top, z0))
		st.add_vertex(Vector3(xr, y_bot, z0))
		st.add_vertex(Vector3(x0, y_bot, z0))
		st.add_vertex(Vector3(xr, y_top, z0))
		st.add_vertex(Vector3(x0, y_bot, z0))
		st.add_vertex(Vector3(x0, y_top, z0))
