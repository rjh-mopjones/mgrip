extends RefCounted
class_name VoxelMeshBuilder

## Builds Minecraft-style voxel terrain meshes from a BiomeMap chunk.
##
## Land columns:  solid top + greedy-merged side faces with substrate blending.
## Ocean columns: no solid geometry — a separate flat water quad at SEA_LEVEL_Y.
## Split into 64 sub-meshes (8×8 of 64×64 blocks) to keep draw calls sane.

const CHUNK_SIZE   := 512
const HEIGHT_SCALE := 256.0
const SUB_SIZE     := 64
const GRID_COUNT: int = CHUNK_SIZE / SUB_SIZE  # 8

## Sea-level in block-space: floor(-0.01 * 256) = -3
const SEA_LEVEL_Y  := -3

## Geological substrate color blended into cliff faces as depth increases.
const SUBSTRATE := Color(0.45, 0.35, 0.28)

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

	var land_mat := StandardMaterial3D.new()
	land_mat.vertex_color_use_as_albedo = true
	land_mat.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL
	land_mat.roughness    = 0.95
	land_mat.metallic     = 0.0

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

	# Pass 1: top faces (one quad per column)
	for lz in SUB_SIZE:
		for lx in SUB_SIZE:
			var x   := ox + lx
			var z   := oz + lz
			var idx := z * CHUNK_SIZE + x
			if ocean_mask[idx]:
				continue
			var y   := heights[idx]
			var bi  := idx * 4
			var col := Color(
				biome_rgba[bi]     / 255.0,
				biome_rgba[bi + 1] / 255.0,
				biome_rgba[bi + 2] / 255.0,
			)
			_top_face(st, x, y, z, col)
			has_geo = true

	# Pass 2: greedy-merged side faces with geological substrate blending
	_build_greedy_sides(st, heights, biome_rgba, ocean_mask, ox, oz)

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

# ── Greedy side meshing ───────────────────────────────────────────────────────
#
# For each of the 4 cardinal directions, scan all cells in the sub-chunk.
# Adjacent cells with the same top_y, bottom_y, and surface color are merged
# into a single wide quad. This cuts side face count by 60-80% on typical
# terrain and eliminates per-block edge noise on cliff faces.

func _build_greedy_sides(
		st: SurfaceTool,
		heights: PackedInt32Array,
		biome_rgba: PackedByteArray,
		ocean_mask: PackedByteArray,
		ox: int, oz: int) -> void:
	# directions: [dx, dz] — 4 cardinal faces
	var dirs := [[1, 0], [-1, 0], [0, 1], [0, -1]]

	for dir in dirs:
		var dx    : int = dir[0]
		var dz    : int = dir[1]
		# For dx faces, the quad spans the z axis → merge along z (lz).
		# For dz faces, the quad spans the x axis → merge along x (lx).
		var merge_z := dx != 0

		var visited := PackedByteArray()
		visited.resize(SUB_SIZE * SUB_SIZE)
		visited.fill(0)

		for lz in SUB_SIZE:
			for lx in SUB_SIZE:
				if visited[lz * SUB_SIZE + lx]:
					continue

				var x   := ox + lx
				var z   := oz + lz
				var idx := z * CHUNK_SIZE + x

				if ocean_mask[idx]:
					visited[lz * SUB_SIZE + lx] = 1
					continue

				var y   := heights[idx]
				var nx  := x + dx
				var nz  := z + dz
				var ny  : int

				if nx < 0 or nx >= CHUNK_SIZE or nz < 0 or nz >= CHUNK_SIZE:
					ny = y - 1
				else:
					var nidx := nz * CHUNK_SIZE + nx
					ny = SEA_LEVEL_Y - 1 if ocean_mask[nidx] else heights[nidx]

				if ny >= y:
					visited[lz * SUB_SIZE + lx] = 1
					continue

				var bi := idx * 4
				var r0 : int = biome_rgba[bi]
				var g0 : int = biome_rgba[bi + 1]
				var b0 : int = biome_rgba[bi + 2]

				# Extend run along the merge axis, requiring matching y, ny, color
				var run := 1
				if merge_z:
					while lz + run < SUB_SIZE:
						var nlz   := lz + run
						var x2    := ox + lx
						var z2    := oz + nlz
						var idx2  := z2 * CHUNK_SIZE + x2
						if visited[nlz * SUB_SIZE + lx]:
							break
						if ocean_mask[idx2] or heights[idx2] != y:
							break
						var nx2   := x2 + dx
						var nny   : int
						if nx2 < 0 or nx2 >= CHUNK_SIZE:
							nny = y - 1
						else:
							var nidx2 := z2 * CHUNK_SIZE + nx2
							nny = SEA_LEVEL_Y - 1 if ocean_mask[nidx2] else heights[nidx2]
						if nny != ny:
							break
						var bi2 := idx2 * 4
						if biome_rgba[bi2] != r0 or biome_rgba[bi2+1] != g0 or biome_rgba[bi2+2] != b0:
							break
						run += 1
					for r in run:
						visited[(lz + r) * SUB_SIZE + lx] = 1
				else:
					while lx + run < SUB_SIZE:
						var nlx   := lx + run
						var x2    := ox + nlx
						var z2    := oz + lz
						var idx2  := z2 * CHUNK_SIZE + x2
						if visited[lz * SUB_SIZE + nlx]:
							break
						if ocean_mask[idx2] or heights[idx2] != y:
							break
						var nz2   := z2 + dz
						var nny   : int
						if nz2 < 0 or nz2 >= CHUNK_SIZE:
							nny = y - 1
						else:
							var nidx2 := nz2 * CHUNK_SIZE + x2
							nny = SEA_LEVEL_Y - 1 if ocean_mask[nidx2] else heights[nidx2]
						if nny != ny:
							break
						var bi2 := idx2 * 4
						if biome_rgba[bi2] != r0 or biome_rgba[bi2+1] != g0 or biome_rgba[bi2+2] != b0:
							break
						run += 1
					for r in run:
						visited[lz * SUB_SIZE + (lx + r)] = 1

				# Blend surface color toward geological substrate as cliff depth increases
				var surface_col := Color(r0 / 255.0, g0 / 255.0, b0 / 255.0)
				var exposure    := float(y + 1) - float(ny + 1)
				var t           := clampf((exposure - 2.0) / 4.0, 0.0, 1.0)
				var face_col    := surface_col.lerp(SUBSTRATE, t * 0.7)

				_emit_side_face(st, x, y, ny, z, dx, dz, run, face_col)

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
