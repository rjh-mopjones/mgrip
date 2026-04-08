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
## Returns heights and ocean_mask so world.gd can use them for spawn + collision.
func build_terrain(biome_map: MgBiomeMap, parent: Node3D, lod_name: String = GenerationManager.LOD0_NAME) -> Dictionary:
	var config := GenerationManager.runtime_chunk_config_for_lod(lod_name)
	var mesh_data := biome_map.build_chunk_mesh_data(
		HEIGHT_SCALE,
		int(config["sub_size"]),
		bool(config["use_edge_skirts"]),
	)
	return build_terrain_from_mesh_data(mesh_data, parent)

func build_terrain_from_mesh_data(mesh_data: Dictionary, parent: Node3D) -> Dictionary:
	var heights: PackedInt32Array = mesh_data.get("heights", PackedInt32Array())
	var ocean_mask: PackedByteArray = mesh_data.get("ocean_mask", PackedByteArray())
	var land_surfaces: Array = mesh_data.get("land_surfaces", [])
	var water_surfaces: Array = mesh_data.get("water_surfaces", [])

	var land_mat := ShaderMaterial.new()
	land_mat.shader = preload("res://assets/shaders/terrain.gdshader")

	var water_mat := ShaderMaterial.new()
	water_mat.shader = preload("res://assets/shaders/water.gdshader")

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

	return {"heights": heights, "ocean_mask": ocean_mask}

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
