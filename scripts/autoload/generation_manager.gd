extends Node

## Wraps MgTerrainGen.
## Runtime chunk generation is synchronous for now — threading can be added later.

const BLOCKS_PER_CHUNK := 512
const WORLD_UNITS_PER_CHUNK := 1.0
const BLOCKS_PER_WORLD_UNIT := float(BLOCKS_PER_CHUNK) / WORLD_UNITS_PER_CHUNK
const LOD0_NAME := "LOD0"
const LOD0_RESOLUTION := 512
const LOD0_DETAIL_LEVEL := 2
const LOD0_FREQ_SCALE := 8.0
const LOD0_SUB_SIZE := 64
const LOD0_USE_EDGE_SKIRTS := false
const LOD1_NAME := "LOD1"
const LOD1_RESOLUTION := 129
const LOD1_DETAIL_LEVEL := 1
const LOD1_FREQ_SCALE := 8.0
const LOD1_SUB_SIZE := 64
const LOD1_USE_EDGE_SKIRTS := false
const LOD2_NAME := "LOD2"
const LOD2_RESOLUTION := 65
const LOD2_DETAIL_LEVEL := 0
const LOD2_FREQ_SCALE := 8.0
const LOD2_SUB_SIZE := 64
const LOD2_USE_EDGE_SKIRTS := true

var _gen: MgTerrainGen

func _ready() -> void:
	_gen = MgTerrainGen.new()

## Runtime chunk coord -> generator-space world origin.
func chunk_coord_to_world_origin(chunk_coord: Vector2i) -> Vector2:
	return Vector2(
		float(chunk_coord.x) * WORLD_UNITS_PER_CHUNK,
		float(chunk_coord.y) * WORLD_UNITS_PER_CHUNK,
	)

## Generator-space world origin -> runtime chunk coord.
func world_origin_to_chunk_coord(world_x: float, world_y: float) -> Vector2i:
	return Vector2i(
		int(floor(world_x / WORLD_UNITS_PER_CHUNK)),
		int(floor(world_y / WORLD_UNITS_PER_CHUNK)),
	)

## Scene-space block position -> runtime chunk coord.
func scene_block_to_chunk_coord(anchor_chunk: Vector2i, block_x: float, block_z: float) -> Vector2i:
	return anchor_chunk + Vector2i(
		int(floor(block_x / float(BLOCKS_PER_CHUNK))),
		int(floor(block_z / float(BLOCKS_PER_CHUNK))),
	)

## Scene-space block position -> chunk-local block coord in [0, 511].
func scene_block_to_local_block(block_x: float, block_z: float) -> Vector2i:
	var bx := posmod(int(floor(block_x)), BLOCKS_PER_CHUNK)
	var bz := posmod(int(floor(block_z)), BLOCKS_PER_CHUNK)
	return Vector2i(bx, bz)

## Runtime chunk coord -> scene-space origin in block units.
func chunk_coord_to_scene_origin(chunk_coord: Vector2i, anchor_chunk: Vector2i) -> Vector3:
	var dx := (chunk_coord.x - anchor_chunk.x) * BLOCKS_PER_CHUNK
	var dz := (chunk_coord.y - anchor_chunk.y) * BLOCKS_PER_CHUNK
	return Vector3(float(dx), 0.0, float(dz))

## Runtime chunk coord -> generated runtime chunk.
func generate_runtime_chunk(chunk_coord: Vector2i) -> MgBiomeMap:
	var world_origin := chunk_coord_to_world_origin(chunk_coord)
	return _gen.generate_chunk(GameState.world_seed, world_origin.x, world_origin.y)

func generate_runtime_chunk_for_lod(chunk_coord: Vector2i, lod_name: String) -> MgBiomeMap:
	var world_origin := chunk_coord_to_world_origin(chunk_coord)
	var config := runtime_chunk_config_for_lod(lod_name)
	return _gen.generate_chunk_lod(
		GameState.world_seed,
		world_origin.x,
		world_origin.y,
		int(config["resolution"]),
		int(config["detail_level"]),
		float(config["freq_scale"]),
	)

func runtime_chunk_config_for_lod(lod_name: String) -> Dictionary:
	match lod_name:
		LOD2_NAME:
			return {
				"resolution": LOD2_RESOLUTION,
				"detail_level": LOD2_DETAIL_LEVEL,
				"freq_scale": LOD2_FREQ_SCALE,
				"sub_size": LOD2_SUB_SIZE,
				"use_edge_skirts": LOD2_USE_EDGE_SKIRTS,
			}
		LOD1_NAME:
			return {
				"resolution": LOD1_RESOLUTION,
				"detail_level": LOD1_DETAIL_LEVEL,
				"freq_scale": LOD1_FREQ_SCALE,
				"sub_size": LOD1_SUB_SIZE,
				"use_edge_skirts": LOD1_USE_EDGE_SKIRTS,
			}
		_:
			return {
				"resolution": LOD0_RESOLUTION,
				"detail_level": LOD0_DETAIL_LEVEL,
				"freq_scale": LOD0_FREQ_SCALE,
				"sub_size": LOD0_SUB_SIZE,
				"use_edge_skirts": LOD0_USE_EDGE_SKIRTS,
			}

## Region tile coord -> meso-scale region tile.
func generate_region_tile(region_coord: Vector2i) -> MgBiomeMap:
	return _gen.generate_meso_tile(GameState.world_seed, region_coord.x, region_coord.y)

## Backwards-compatible wrapper for callers that still pass generator-space origins.
func generate_chunk(world_x: float, world_y: float) -> MgBiomeMap:
	return _gen.generate_chunk(GameState.world_seed, world_x, world_y)
