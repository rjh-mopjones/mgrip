extends RefCounted
class_name VoxelMeshBuilder

## Builds Minecraft-style voxel terrain meshes from a BiomeMap chunk.
##
## The heavy mesh-data generation now lives in Rust; this script only assembles
## ArrayMesh instances and attaches them with the right materials.

const CHUNK_SIZE   := 512
const HEIGHT_SCALE := 200.0
const SEA_LEVEL_Y  := -2

## Build terrain + water meshes and attach to parent.
## Returns heights and fluid_surface_mask so world.gd can use them for spawn + landing.
func build_terrain(
		biome_map: MgBiomeMap,
		parent: Node3D,
		lod_name: String = GenerationManager.LOD0_NAME,
		runtime_presentation: Dictionary = {},
		runtime_presentation_grids: Dictionary = {}) -> Dictionary:
	var config := GenerationManager.runtime_chunk_config_for_lod(lod_name)
	var mesh_data := biome_map.build_chunk_mesh_data(
		HEIGHT_SCALE,
		int(config["sub_size"]),
		bool(config["use_edge_skirts"]),
	)
	return build_terrain_from_mesh_data(
		mesh_data,
		parent,
		lod_name,
		runtime_presentation,
		runtime_presentation_grids,
	)

func build_terrain_from_mesh_data(
		mesh_data: Dictionary,
		parent: Node3D,
		lod_name: String = GenerationManager.LOD0_NAME,
		runtime_presentation: Dictionary = {},
		runtime_presentation_grids: Dictionary = {}) -> Dictionary:
	var heights: PackedInt32Array = mesh_data.get("heights", PackedInt32Array())
	var collision_heights: PackedFloat32Array = mesh_data.get("collision_heights", PackedFloat32Array())
	var fluid_surface_mask: PackedByteArray = mesh_data.get("fluid_surface_mask", PackedByteArray())
	var land_surfaces: Array = mesh_data.get("land_surfaces", [])
	var water_surfaces: Array = mesh_data.get("water_surfaces", [])

	if _is_headless_runtime():
		return {
			"heights": heights,
			"collision_heights": collision_heights,
			"fluid_surface_mask": fluid_surface_mask,
		}

	var land_mat := ShaderMaterial.new()
	land_mat.shader = preload("res://assets/shaders/terrain.gdshader")
	_configure_land_material(
		land_mat,
		lod_name,
		runtime_presentation,
		runtime_presentation_grids,
	)

	var water_mat := ShaderMaterial.new()
	water_mat.shader = preload("res://assets/shaders/water.gdshader")
	_configure_water_material(water_mat, runtime_presentation, runtime_presentation_grids)

	for surface in land_surfaces:
		var land_mesh := _build_surface_mesh(surface, true)
		if land_mesh == null:
			continue
		var mi := MeshInstance3D.new()
		mi.mesh = land_mesh
		mi.material_override = land_mat
		parent.add_child(mi)

	for surface in water_surfaces:
		var water_mesh := _build_surface_mesh(surface, false)
		if water_mesh == null:
			continue
		var mi := MeshInstance3D.new()
		mi.mesh = water_mesh
		mi.material_override = water_mat
		parent.add_child(mi)

	return {
		"heights": heights,
		"collision_heights": collision_heights,
		"fluid_surface_mask": fluid_surface_mask,
	}

func _is_headless_runtime() -> bool:
	return DisplayServer.get_name() == "headless"

func _configure_land_material(
		material: ShaderMaterial,
		lod_name: String,
		runtime_presentation: Dictionary,
		runtime_presentation_grids: Dictionary) -> void:
	var haze_start := 700.0
	var haze_end := 2400.0
	var lod_blend_strength := 0.0
	match lod_name:
		GenerationManager.LOD1_NAME:
			haze_start = 520.0
			haze_end = 2200.0
			lod_blend_strength = 0.38
		GenerationManager.LOD2_NAME:
			haze_start = 360.0
			haze_end = 1800.0
			lod_blend_strength = 0.72
	material.set_shader_parameter("distance_haze_start", haze_start)
	material.set_shader_parameter("distance_haze_end", haze_end)
	material.set_shader_parameter("lod_blend_strength", lod_blend_strength)
	var atmosphere_name := _enum_name(
		runtime_presentation.get("atmosphere_class", {}),
		"TemperateTwilight"
	)
	material.set_shader_parameter(
		"horizon_tint",
		_atmosphere_horizon_tint(atmosphere_name)
	)
	var palette_name := _enum_name(
		runtime_presentation.get("surface_palette_class", {}),
		"ExposedStone"
	)
	var palette_profile := _surface_palette_profile(palette_name)
	material.set_shader_parameter("palette_top_tint", palette_profile["top_tint"])
	material.set_shader_parameter("palette_cliff_tint", palette_profile["cliff_tint"])
	material.set_shader_parameter("palette_shadow_tint", palette_profile["shadow_tint"])
	material.set_shader_parameter("palette_top_strength", float(palette_profile["top_strength"]))
	material.set_shader_parameter("palette_cliff_strength", float(palette_profile["cliff_strength"]))
	material.set_shader_parameter("palette_shadow_strength", float(palette_profile["shadow_strength"]))
	material.set_shader_parameter("palette_dust_strength", float(palette_profile["dust_strength"]))
	material.set_shader_parameter("palette_darkness", float(palette_profile["darkness"]))
	material.set_shader_parameter(
		"average_snowpack",
		clampf(float(runtime_presentation.get("average_snowpack", 0.0)), 0.0, 1.0)
	)
	material.set_shader_parameter(
		"average_water_table",
		clampf(float(runtime_presentation.get("average_water_table", 0.0)), 0.0, 1.0)
	)
	material.set_shader_parameter(
		"average_aridity",
		clampf(float(runtime_presentation.get("average_aridity", 0.0)), 0.0, 1.0)
	)
	var average_temperature := float(runtime_presentation.get("average_temperature", 0.0))
	material.set_shader_parameter(
		"palette_coldness",
		clampf((10.0 - average_temperature) / 70.0, 0.0, 1.0)
	)
	var landform_name := _enum_name(
		runtime_presentation.get("landform_class", {}),
		"FlatPlain"
	)
	var landform_profile := _landform_profile(landform_name)
	material.set_shader_parameter(
		"landform_top_variation",
		float(landform_profile["top_variation"])
	)
	material.set_shader_parameter(
		"landform_cliff_variation",
		float(landform_profile["cliff_variation"])
	)
	material.set_shader_parameter(
		"landform_strata_strength",
		float(landform_profile["strata_strength"])
	)
	material.set_shader_parameter(
		"landform_cliff_boost",
		float(landform_profile["cliff_boost"])
	)
	material.set_shader_parameter(
		"landform_shadow_boost",
		float(landform_profile["shadow_boost"])
	)
	material.set_shader_parameter(
		"landform_roughness_boost",
		float(landform_profile["roughness_boost"])
	)
	material.set_shader_parameter(
		"interestingness_boost",
		clampf(float(runtime_presentation.get("interestingness_score", 0.0)), 0.0, 1.0)
	)
	_configure_reduced_land_textures(material, runtime_presentation_grids)

func _atmosphere_horizon_tint(atmosphere_name: String) -> Color:
	match atmosphere_name:
		"BlastedRadiance":
			return Color(0.90, 0.34, 0.14)
		"HarshAmberHaze":
			return Color(0.78, 0.28, 0.14)
		"DryTwilight":
			return Color(0.40, 0.20, 0.18)
		"WetTwilight":
			return Color(0.28, 0.20, 0.20)
		"FrostTwilight":
			return Color(0.40, 0.46, 0.56)
		"PolarGlow":
			return Color(0.24, 0.34, 0.42)
		"BlackIceDark":
			return Color(0.08, 0.12, 0.18)
		"GeothermalNight":
			return Color(0.34, 0.14, 0.12)
		_:
			return Color(0.34, 0.22, 0.24)

func _surface_palette_profile(palette_name: String) -> Dictionary:
	var profile := {
		"top_tint": Color(0.52, 0.50, 0.50),
		"cliff_tint": Color(0.30, 0.30, 0.32),
		"shadow_tint": Color(0.14, 0.14, 0.18),
		"top_strength": 0.30,
		"cliff_strength": 0.34,
		"shadow_strength": 0.22,
		"dust_strength": 0.03,
		"darkness": 0.12,
	}
	match palette_name:
		"ScorchedStone":
			profile = {
				"top_tint": Color(0.62, 0.38, 0.25),
				"cliff_tint": Color(0.24, 0.17, 0.16),
				"shadow_tint": Color(0.12, 0.08, 0.08),
				"top_strength": 0.42,
				"cliff_strength": 0.48,
				"shadow_strength": 0.26,
				"dust_strength": 0.18,
				"darkness": 0.10,
			}
		"AshDust":
			profile = {
				"top_tint": Color(0.56, 0.48, 0.44),
				"cliff_tint": Color(0.22, 0.20, 0.20),
				"shadow_tint": Color(0.10, 0.10, 0.11),
				"top_strength": 0.40,
				"cliff_strength": 0.46,
				"shadow_strength": 0.24,
				"dust_strength": 0.08,
				"darkness": 0.14,
			}
		"DarkTerminusSoil":
			profile = {
				"top_tint": Color(0.20, 0.08, 0.14),
				"cliff_tint": Color(0.10, 0.04, 0.10),
				"shadow_tint": Color(0.03, 0.02, 0.05),
				"top_strength": 0.62,
				"cliff_strength": 0.58,
				"shadow_strength": 0.34,
				"dust_strength": 0.01,
				"darkness": 0.34,
			}
		"WetTerminusGround":
			profile = {
				"top_tint": Color(0.30, 0.18, 0.22),
				"cliff_tint": Color(0.18, 0.11, 0.16),
				"shadow_tint": Color(0.08, 0.05, 0.08),
				"top_strength": 0.34,
				"cliff_strength": 0.36,
				"shadow_strength": 0.22,
				"dust_strength": 0.03,
				"darkness": 0.20,
			}
		"FungalLowland":
			profile = {
				"top_tint": Color(0.24, 0.12, 0.22),
				"cliff_tint": Color(0.14, 0.08, 0.15),
				"shadow_tint": Color(0.06, 0.03, 0.07),
				"top_strength": 0.40,
				"cliff_strength": 0.42,
				"shadow_strength": 0.28,
				"dust_strength": 0.02,
				"darkness": 0.28,
			}
		"CoastalSediment":
			profile = {
				"top_tint": Color(0.70, 0.58, 0.46),
				"cliff_tint": Color(0.42, 0.34, 0.28),
				"shadow_tint": Color(0.20, 0.14, 0.12),
				"top_strength": 0.34,
				"cliff_strength": 0.28,
				"shadow_strength": 0.18,
				"dust_strength": 0.08,
				"darkness": 0.06,
			}
		"SaltCrust":
			profile = {
				"top_tint": Color(0.88, 0.82, 0.74),
				"cliff_tint": Color(0.60, 0.52, 0.44),
				"shadow_tint": Color(0.24, 0.18, 0.16),
				"top_strength": 0.44,
				"cliff_strength": 0.32,
				"shadow_strength": 0.16,
				"dust_strength": 0.04,
				"darkness": 0.04,
			}
		"SnowCover":
			profile = {
				"top_tint": Color(0.92, 0.95, 0.98),
				"cliff_tint": Color(0.50, 0.56, 0.64),
				"shadow_tint": Color(0.14, 0.18, 0.24),
				"top_strength": 0.50,
				"cliff_strength": 0.34,
				"shadow_strength": 0.20,
				"dust_strength": 0.00,
				"darkness": 0.02,
			}
		"BlueIce":
			profile = {
				"top_tint": Color(0.72, 0.84, 0.96),
				"cliff_tint": Color(0.38, 0.50, 0.62),
				"shadow_tint": Color(0.10, 0.16, 0.24),
				"top_strength": 0.48,
				"cliff_strength": 0.42,
				"shadow_strength": 0.20,
				"dust_strength": 0.00,
				"darkness": 0.04,
			}
		"BlackIceRock":
			profile = {
				"top_tint": Color(0.18, 0.24, 0.32),
				"cliff_tint": Color(0.08, 0.12, 0.18),
				"shadow_tint": Color(0.03, 0.04, 0.08),
				"top_strength": 0.32,
				"cliff_strength": 0.36,
				"shadow_strength": 0.30,
				"dust_strength": 0.01,
				"darkness": 0.32,
			}
		"IronOxideHighland":
			profile = {
				"top_tint": Color(0.68, 0.34, 0.26),
				"cliff_tint": Color(0.34, 0.18, 0.16),
				"shadow_tint": Color(0.12, 0.08, 0.08),
				"top_strength": 0.40,
				"cliff_strength": 0.42,
				"shadow_strength": 0.24,
				"dust_strength": 0.14,
				"darkness": 0.10,
			}
		"VegetatedDarkCanopyFloor":
			profile = {
				"top_tint": Color(0.12, 0.04, 0.10),
				"cliff_tint": Color(0.06, 0.02, 0.07),
				"shadow_tint": Color(0.02, 0.01, 0.03),
				"top_strength": 0.64,
				"cliff_strength": 0.58,
				"shadow_strength": 0.36,
				"dust_strength": 0.01,
				"darkness": 0.40,
			}
		"ExposedStone":
			profile = {
				"top_tint": Color(0.56, 0.54, 0.54),
				"cliff_tint": Color(0.34, 0.34, 0.36),
				"shadow_tint": Color(0.16, 0.16, 0.20),
				"top_strength": 0.28,
				"cliff_strength": 0.34,
				"shadow_strength": 0.22,
				"dust_strength": 0.02,
				"darkness": 0.14,
			}
	return profile

func _landform_profile(landform_name: String) -> Dictionary:
	var profile := {
		"top_variation": 0.42,
		"cliff_variation": 0.38,
		"strata_strength": 0.32,
		"cliff_boost": 0.12,
		"shadow_boost": 0.10,
		"roughness_boost": 0.04,
	}
	match landform_name:
		"FlatPlain":
			profile = {
				"top_variation": 0.20,
				"cliff_variation": 0.12,
				"strata_strength": 0.10,
				"cliff_boost": 0.02,
				"shadow_boost": 0.02,
				"roughness_boost": 0.00,
			}
		"Basin":
			profile = {
				"top_variation": 0.26,
				"cliff_variation": 0.18,
				"strata_strength": 0.14,
				"cliff_boost": 0.04,
				"shadow_boost": 0.06,
				"roughness_boost": 0.02,
			}
		"Plateau":
			profile = {
				"top_variation": 0.36,
				"cliff_variation": 0.26,
				"strata_strength": 0.34,
				"cliff_boost": 0.10,
				"shadow_boost": 0.08,
				"roughness_boost": 0.04,
			}
		"Ridge":
			profile = {
				"top_variation": 0.58,
				"cliff_variation": 0.62,
				"strata_strength": 0.42,
				"cliff_boost": 0.20,
				"shadow_boost": 0.16,
				"roughness_boost": 0.08,
			}
		"Escarpment":
			profile = {
				"top_variation": 0.46,
				"cliff_variation": 0.82,
				"strata_strength": 0.74,
				"cliff_boost": 0.28,
				"shadow_boost": 0.22,
				"roughness_boost": 0.10,
			}
		"BrokenHighland":
			profile = {
				"top_variation": 0.74,
				"cliff_variation": 0.70,
				"strata_strength": 0.48,
				"cliff_boost": 0.22,
				"shadow_boost": 0.18,
				"roughness_boost": 0.10,
			}
		"AlpineMassif":
			profile = {
				"top_variation": 0.82,
				"cliff_variation": 0.86,
				"strata_strength": 0.58,
				"cliff_boost": 0.28,
				"shadow_boost": 0.24,
				"roughness_boost": 0.12,
			}
		"CoastShelf":
			profile = {
				"top_variation": 0.28,
				"cliff_variation": 0.18,
				"strata_strength": 0.16,
				"cliff_boost": 0.04,
				"shadow_boost": 0.04,
				"roughness_boost": 0.02,
			}
		"CliffCoast":
			profile = {
				"top_variation": 0.44,
				"cliff_variation": 0.84,
				"strata_strength": 0.64,
				"cliff_boost": 0.28,
				"shadow_boost": 0.22,
				"roughness_boost": 0.08,
			}
		"FrozenShelf":
			profile = {
				"top_variation": 0.22,
				"cliff_variation": 0.28,
				"strata_strength": 0.18,
				"cliff_boost": 0.08,
				"shadow_boost": 0.06,
				"roughness_boost": 0.06,
			}
		"DuneWaste":
			profile = {
				"top_variation": 0.62,
				"cliff_variation": 0.18,
				"strata_strength": 0.86,
				"cliff_boost": 0.06,
				"shadow_boost": 0.08,
				"roughness_boost": 0.02,
			}
		"Badlands":
			profile = {
				"top_variation": 0.78,
				"cliff_variation": 0.76,
				"strata_strength": 0.84,
				"cliff_boost": 0.24,
				"shadow_boost": 0.18,
				"roughness_boost": 0.10,
			}
		"FractureBelt":
			profile = {
				"top_variation": 0.76,
				"cliff_variation": 0.82,
				"strata_strength": 0.56,
				"cliff_boost": 0.26,
				"shadow_boost": 0.20,
				"roughness_boost": 0.12,
			}
		"RiverCutLowland":
			profile = {
				"top_variation": 0.38,
				"cliff_variation": 0.26,
				"strata_strength": 0.24,
				"cliff_boost": 0.08,
				"shadow_boost": 0.08,
				"roughness_boost": 0.04,
			}
		"VolcanicField":
			profile = {
				"top_variation": 0.80,
				"cliff_variation": 0.74,
				"strata_strength": 0.72,
				"cliff_boost": 0.24,
				"shadow_boost": 0.18,
				"roughness_boost": 0.14,
			}
	return profile

func _configure_reduced_land_textures(
		material: ShaderMaterial,
		runtime_presentation_grids: Dictionary) -> void:
	var landform_grid: Dictionary = runtime_presentation_grids.get("landform_grid", {})
	var surface_palette_grid: Dictionary = runtime_presentation_grids.get("surface_palette_grid", {})
	if landform_grid.is_empty() or surface_palette_grid.is_empty():
		material.set_shader_parameter("use_reduced_presentation_textures", false)
		return
	var landform_texture := _build_landform_grid_texture(landform_grid)
	var palette_weight_texture := _build_palette_weight_grid_texture(surface_palette_grid)
	var palette_tint_texture := _build_palette_tint_grid_texture(surface_palette_grid)
	if landform_texture == null or palette_weight_texture == null or palette_tint_texture == null:
		material.set_shader_parameter("use_reduced_presentation_textures", false)
		return
	material.set_shader_parameter("use_reduced_presentation_textures", true)
	material.set_shader_parameter("landform_grid_texture", landform_texture)
	material.set_shader_parameter("palette_weight_grid_texture", palette_weight_texture)
	material.set_shader_parameter("palette_tint_grid_texture", palette_tint_texture)

func _build_landform_grid_texture(grid: Dictionary) -> Texture2D:
	var width := int(grid.get("width", 0))
	var height := int(grid.get("height", 0))
	var ids: PackedByteArray = grid.get("ids", PackedByteArray())
	if width <= 0 or height <= 0 or ids.size() != width * height:
		return null
	var lookup := _grid_legend_lookup(grid)
	var image := Image.create(width, height, false, Image.FORMAT_RGBA8)
	for y in height:
		for x in width:
			var index := y * width + x
			var landform_name := String(lookup.get(int(ids[index]), "FlatPlain"))
			var profile := _landform_profile(landform_name)
			image.set_pixel(
				x,
				y,
				Color(
					clampf(float(profile["top_variation"]), 0.0, 1.0),
					clampf(float(profile["cliff_variation"]), 0.0, 1.0),
					clampf(float(profile["strata_strength"]), 0.0, 1.0),
					clampf(
						(float(profile["cliff_boost"]) + float(profile["shadow_boost"])) * 2.0,
						0.0,
						1.0
					),
				),
			)
	return ImageTexture.create_from_image(image)

func _build_palette_weight_grid_texture(grid: Dictionary) -> Texture2D:
	var width := int(grid.get("width", 0))
	var height := int(grid.get("height", 0))
	var ids: PackedByteArray = grid.get("ids", PackedByteArray())
	if width <= 0 or height <= 0 or ids.size() != width * height:
		return null
	var lookup := _grid_legend_lookup(grid)
	var image := Image.create(width, height, false, Image.FORMAT_RGBA8)
	for y in height:
		for x in width:
			var index := y * width + x
			var palette_name := String(lookup.get(int(ids[index]), "ExposedStone"))
			var profile := _surface_palette_profile(palette_name)
			image.set_pixel(
				x,
				y,
				Color(
					clampf(float(profile["top_strength"]), 0.0, 1.0),
					clampf(float(profile["cliff_strength"]), 0.0, 1.0),
					clampf(float(profile["dust_strength"]), 0.0, 1.0),
					clampf(float(profile["darkness"]) * 3.0, 0.0, 1.0),
				),
			)
	return ImageTexture.create_from_image(image)

func _build_palette_tint_grid_texture(grid: Dictionary) -> Texture2D:
	var width := int(grid.get("width", 0))
	var height := int(grid.get("height", 0))
	var ids: PackedByteArray = grid.get("ids", PackedByteArray())
	if width <= 0 or height <= 0 or ids.size() != width * height:
		return null
	var lookup := _grid_legend_lookup(grid)
	var image := Image.create(width, height, false, Image.FORMAT_RGBA8)
	for y in height:
		for x in width:
			var index := y * width + x
			var palette_name := String(lookup.get(int(ids[index]), "ExposedStone"))
			var profile := _surface_palette_profile(palette_name)
			var top_tint: Color = profile["top_tint"]
			var cliff_tint: Color = profile["cliff_tint"]
			var shadow_tint: Color = profile["shadow_tint"]
			var local_tint := top_tint.lerp(cliff_tint, 0.35).lerp(shadow_tint, 0.18)
			image.set_pixel(
				x,
				y,
				Color(
					local_tint.r,
					local_tint.g,
					local_tint.b,
					clampf(float(profile["shadow_strength"]), 0.0, 1.0),
				),
			)
	return ImageTexture.create_from_image(image)

func _configure_water_material(
		material: ShaderMaterial,
		runtime_presentation: Dictionary,
		runtime_presentation_grids: Dictionary) -> void:
	var water_state := _enum_name(runtime_presentation.get("water_state", {}), "LiquidSea")
	match water_state:
		"FrozenSea":
			_set_water_profile(
				material,
				Color(0.66, 0.76, 0.86),
				Color(0.38, 0.48, 0.60),
				0.10,
				0.02,
				0.88,
				0.96,
				0.68,
				0.22,
				0.02,
			)
		"IceSheet":
			_set_water_profile(
				material,
				Color(0.86, 0.90, 0.96),
				Color(0.64, 0.72, 0.82),
				0.02,
				0.0,
				0.94,
				0.99,
				0.82,
				0.16,
				0.0,
			)
		"BrineFlat":
			_set_water_profile(
				material,
				Color(0.60, 0.49, 0.38),
				Color(0.44, 0.33, 0.26),
				0.04,
				0.01,
				0.86,
				0.94,
				0.78,
				0.18,
				0.0,
			)
		"EvaporiteBasin":
			_set_water_profile(
				material,
				Color(0.84, 0.79, 0.72),
				Color(0.67, 0.61, 0.54),
				0.01,
				0.0,
				0.92,
				0.98,
				0.86,
				0.12,
				0.0,
			)
		"LiquidCoast":
			_set_water_profile(
				material,
				Color(0.24, 0.24, 0.38),
				Color(0.09, 0.10, 0.20),
				0.55,
				0.09,
				0.48,
				0.86,
				0.24,
				0.44,
				0.08,
			)
		_:
			_set_water_profile(
				material,
				Color(0.16, 0.20, 0.42),
				Color(0.04, 0.05, 0.16),
				0.80,
				0.15,
				0.45,
				0.88,
				0.15,
				0.50,
				0.10,
			)
	_configure_reduced_water_texture(material, runtime_presentation_grids)

func _configure_reduced_water_texture(
		material: ShaderMaterial,
		runtime_presentation_grids: Dictionary) -> void:
	var water_state_grid: Dictionary = runtime_presentation_grids.get("water_state_grid", {})
	if water_state_grid.is_empty():
		material.set_shader_parameter("use_reduced_water_grid", false)
		return
	var water_texture := _build_water_state_grid_texture(water_state_grid)
	if water_texture == null:
		material.set_shader_parameter("use_reduced_water_grid", false)
		return
	material.set_shader_parameter("use_reduced_water_grid", true)
	material.set_shader_parameter("water_state_grid_texture", water_texture)

func _build_water_state_grid_texture(grid: Dictionary) -> Texture2D:
	var width := int(grid.get("width", 0))
	var height := int(grid.get("height", 0))
	var ids: PackedByteArray = grid.get("ids", PackedByteArray())
	if width <= 0 or height <= 0 or ids.size() != width * height:
		return null
	var lookup := _grid_legend_lookup(grid)
	var image := Image.create(width, height, false, Image.FORMAT_RGBA8)
	for y in height:
		for x in width:
			var index := y * width + x
			var water_state_name := String(lookup.get(int(ids[index]), "LiquidSea"))
			image.set_pixel(x, y, _water_state_profile_color(water_state_name))
	return ImageTexture.create_from_image(image)

func _water_state_profile_color(water_state_name: String) -> Color:
	match water_state_name:
		"FrozenSea":
			return Color(0.86, 0.04, 0.08, 0.20)
		"IceSheet":
			return Color(1.00, 0.02, 0.00, 0.10)
		"BrineFlat":
			return Color(0.10, 0.94, 0.08, 0.24)
		"EvaporiteBasin":
			return Color(0.04, 1.00, 0.04, 0.16)
		"LiquidCoast":
			return Color(0.00, 0.18, 0.78, 0.88)
		"LiquidRiver":
			return Color(0.00, 0.04, 0.62, 0.94)
		"FrozenRiver":
			return Color(0.76, 0.06, 0.22, 0.92)
		"MeltwaterChannel":
			return Color(0.28, 0.12, 0.46, 0.86)
		"MarshWater":
			return Color(0.08, 0.24, 0.34, 0.72)
		_:
			return Color(0.00, 0.04, 1.00, 0.14)

func _set_water_profile(
		material: ShaderMaterial,
		shallow_color: Color,
		deep_color: Color,
		wave_speed: float,
		wave_height: float,
		alpha_min: float,
		alpha_max: float,
		roughness_value: float,
		specular_value: float,
		metallic_value: float) -> void:
	material.set_shader_parameter("shallow_color", shallow_color)
	material.set_shader_parameter("deep_color", deep_color)
	material.set_shader_parameter("wave_speed", wave_speed)
	material.set_shader_parameter("wave_height", wave_height)
	material.set_shader_parameter("alpha_min", alpha_min)
	material.set_shader_parameter("alpha_max", alpha_max)
	material.set_shader_parameter("roughness_value", roughness_value)
	material.set_shader_parameter("specular_value", specular_value)
	material.set_shader_parameter("metallic_value", metallic_value)

func _grid_legend_lookup(grid: Dictionary) -> Dictionary:
	var lookup := {}
	var legend: Array = grid.get("legend", [])
	for entry in legend:
		if entry is Dictionary:
			lookup[int(entry.get("id", -1))] = String(entry.get("name", ""))
	return lookup

func _enum_name(value, fallback: String) -> String:
	if value is Dictionary:
		return String((value as Dictionary).get("name", fallback))
	return fallback

func _build_surface_mesh(surface: Dictionary, include_colors: bool) -> ArrayMesh:
	var vertices: PackedVector3Array = surface.get("vertices", PackedVector3Array())
	var normals: PackedVector3Array = surface.get("normals", PackedVector3Array())
	var indices: PackedInt32Array = surface.get("indices", PackedInt32Array())
	if vertices.is_empty() or indices.is_empty():
		return null

	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_INDEX] = indices
	if include_colors:
		arrays[Mesh.ARRAY_COLOR] = surface.get("colors", PackedColorArray())

	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh
