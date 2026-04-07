extends Node

## Wraps MgTerrainGen and caches recently-generated chunks.
## All generation is synchronous for now — threading can be added later.

var _gen: MgTerrainGen

func _ready() -> void:
	_gen = MgTerrainGen.new()

## Generate a 512×512 meso tile at chunk grid position (chunk_x, chunk_y).
## Each chunk covers 64×64 world units — wide enough for continents and oceans.
## The 16×8 macro grid covers the full 1024×512 world.
func generate_chunk(world_x: float, world_y: float) -> MgBiomeMap:
	return _gen.generate_chunk(GameState.world_seed, world_x, world_y)
