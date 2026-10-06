extends Node3D
## Marks on the ground for what LifeGen placed (spec 013, stage 6): a post
## with its name at each settlement, and roads as strips laid over the
## terrain between the chunks they pass through. Built per chunk as chunks
## load, from the civ pack, and torn down when they unload.
##
## This is the first slice of LifeGen in the game: marks, not buildings.

## The map joins east to west after this many chunks.
const MACRO_CHUNKS_WIDE := 1024
const SETTLEMENT_POST_HEIGHTS := {
	"Metropolis": 40.0, "City": 32.0, "Town": 24.0, "Village": 16.0, "Outpost": 12.0, "Ruins": 8.0,
}
const SETTLEMENT_COLOURS := {
	"Metropolis": Color(1.0, 1.0, 1.0), "City": Color(1.0, 0.84, 0.47), "Town": Color(0.94, 0.67, 0.43),
	"Village": Color(0.88, 0.84, 0.92), "Outpost": Color(0.59, 0.55, 0.69), "Ruins": Color(0.67, 0.27, 0.31),
}
const ROAD_WIDTHS := {"Highway": 6.0, "Road": 4.0, "Trail": 2.5}
const ROAD_COLOURS := {
	"Highway": Color(0.93, 0.90, 0.82), "Road": Color(0.78, 0.68, 0.52), "Trail": Color(0.45, 0.38, 0.33),
}
## A road strip follows the surface sampled every this many blocks.
const ROAD_SAMPLE_BLOCKS := 16
## Marks float this far above the surface so they are not buried in it.
const ABOVE_SURFACE := 0.4
## Chunks this many away from the player, and closer, get marks.
const MARK_RADIUS_CHUNKS := 2

var _world: Node3D
var _anchor_chunk: Vector2i
## One material per colour, shared by every mark of that colour.
var _materials: Dictionary = {}
## Chunk coord -> the node holding its marks.
var _marked: Dictionary = {}

func setup(world: Node3D, anchor_chunk: Vector2i) -> void:
	_world = world
	_anchor_chunk = anchor_chunk

## Build marks for loaded chunks near the player and drop marks for chunks
## that have unloaded or fallen out of range.
func update(current_chunk: Vector2i) -> void:
	var wanted: Dictionary = {}
	for dy in range(-MARK_RADIUS_CHUNKS, MARK_RADIUS_CHUNKS + 1):
		for dx in range(-MARK_RADIUS_CHUNKS, MARK_RADIUS_CHUNKS + 1):
			var chunk := current_chunk + Vector2i(dx, dy)
			if _world.get_chunk_state(chunk).get("has_height_data", false):
				wanted[chunk] = true
	for chunk in _marked.keys():
		if not wanted.has(chunk):
			_marked[chunk].queue_free()
			_marked.erase(chunk)
	for chunk in wanted.keys():
		if not _marked.has(chunk):
			_mark_chunk(chunk)

func _mark_chunk(chunk: Vector2i) -> void:
	var holder := Node3D.new()
	holder.name = "CivMarks_%d_%d" % [chunk.x, chunk.y]
	add_child(holder)
	_marked[chunk] = holder
	var civ: Dictionary = GenerationManager.civ_in_chunk(chunk)
	var origin: Vector3 = GenerationManager.chunk_coord_to_scene_origin(chunk, _anchor_chunk)
	var centre := origin + Vector3(GenerationManager.BLOCKS_PER_CHUNK * 0.5, 0.0, GenerationManager.BLOCKS_PER_CHUNK * 0.5)
	for road in civ.get("roads", []):
		_add_road(holder, chunk, road)
	if civ.get("bridge", false):
		_add_bridge(holder, centre)
	for settlement in civ.get("settlements", []):
		_add_settlement_post(holder, centre, settlement)

## A post at the chunk's centre, taller for a larger settlement, with the
## name above it.
func _add_settlement_post(holder: Node3D, centre: Vector3, settlement: Dictionary) -> void:
	var size: String = settlement.get("size", "Outpost")
	var height: float = SETTLEMENT_POST_HEIGHTS.get(size, 12.0)
	var colour: Color = SETTLEMENT_COLOURS.get(size, Color.WHITE)
	var ground := _surface_at(centre.x, centre.z)
	var post := MeshInstance3D.new()
	var mesh := BoxMesh.new()
	mesh.size = Vector3(2.0, height, 2.0)
	post.mesh = mesh
	post.material_override = _flat_material(colour)
	post.position = Vector3(centre.x, ground + height * 0.5, centre.z)
	holder.add_child(post)
	var label := Label3D.new()
	label.text = "%s\n%s" % [settlement.get("name", ""), size.to_lower()]
	label.font_size = 96
	label.pixel_size = 0.05
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.no_depth_test = true
	label.modulate = colour
	label.outline_modulate = Color(0.06, 0.05, 0.15)
	label.outline_size = 24
	label.position = Vector3(centre.x, ground + height + 6.0, centre.z)
	holder.add_child(label)

## A low dark slab where a road crosses a river.
func _add_bridge(holder: Node3D, centre: Vector3) -> void:
	var slab := MeshInstance3D.new()
	var mesh := BoxMesh.new()
	mesh.size = Vector3(10.0, 1.0, 10.0)
	slab.mesh = mesh
	slab.material_override = _flat_material(Color(0.24, 0.17, 0.12))
	slab.position = Vector3(centre.x, _surface_at(centre.x, centre.z) + 0.6, centre.z)
	holder.add_child(slab)

## The part of a straight stretch of road that lies in this chunk, as a strip
## over the ground. The stretch runs between the centres of two chunks that
## may be far apart; each chunk draws its own part, so the strip is laid
## in short pieces that follow the surface.
func _add_road(holder: Node3D, chunk: Vector2i, road: Dictionary) -> void:
	var from: Vector2i = road.get("from", chunk)
	var to: Vector2i = road.get("to", chunk)
	# The stretch may be given round the seam: bring it near this chunk.
	var shift := 0
	if abs(from.x - chunk.x) > MACRO_CHUNKS_WIDE / 2:
		shift = MACRO_CHUNKS_WIDE if from.x < chunk.x else -MACRO_CHUNKS_WIDE
	var a := _chunk_centre_scene(Vector2i(from.x + shift, from.y))
	var b := _chunk_centre_scene(Vector2i(to.x + shift, to.y))
	var origin: Vector3 = GenerationManager.chunk_coord_to_scene_origin(chunk, _anchor_chunk)
	var side := float(GenerationManager.BLOCKS_PER_CHUNK)
	# Kept a little inside the chunk, so every height sample lands in it and
	# not in a neighbour that may not be loaded yet.
	var inset := 0.5
	var clipped := _clip_to_box(a, b, origin.x + inset, origin.z + inset, origin.x + side - inset, origin.z + side - inset)
	if clipped.is_empty():
		return
	var kind: String = road.get("kind", "Trail")
	var width: float = ROAD_WIDTHS.get(kind, 2.5)
	_add_strip(holder, clipped[0], clipped[1], width, _flat_material(ROAD_COLOURS.get(kind, Color.GRAY)))

## The part of the line a to b inside the box (Liang-Barsky), or [] if none.
func _clip_to_box(a: Vector3, b: Vector3, min_x: float, min_z: float, max_x: float, max_z: float) -> Array:
	var t0 := 0.0
	var t1 := 1.0
	var dx := b.x - a.x
	var dz := b.z - a.z
	for edge in [[-dx, a.x - min_x], [dx, max_x - a.x], [-dz, a.z - min_z], [dz, max_z - a.z]]:
		var p: float = edge[0]
		var q: float = edge[1]
		if p == 0.0:
			if q < 0.0:
				return []
			continue
		var r := q / p
		if p < 0.0:
			t0 = maxf(t0, r)
		else:
			t1 = minf(t1, r)
		if t0 > t1:
			return []
	return [a.lerp(b, t0), a.lerp(b, t1)]

func _add_strip(holder: Node3D, from: Vector3, to: Vector3, width: float, material: Material) -> void:
	var length := Vector2(to.x - from.x, to.z - from.z).length()
	if length < 1.0:
		return
	var pieces: int = max(1, int(ceil(length / ROAD_SAMPLE_BLOCKS)))
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	var across := Vector3(-(to.z - from.z), 0.0, to.x - from.x).normalized() * width * 0.5
	var previous_left := Vector3.ZERO
	var previous_right := Vector3.ZERO
	for i in range(pieces + 1):
		var t := float(i) / float(pieces)
		var p := from.lerp(to, t)
		var y := _surface_at(p.x, p.z) + ABOVE_SURFACE
		var left := Vector3(p.x, y, p.z) - across
		var right := Vector3(p.x, y, p.z) + across
		if i > 0:
			for v in [previous_left, previous_right, left, left, previous_right, right]:
				surface.set_normal(Vector3.UP)
				surface.add_vertex(v)
		previous_left = left
		previous_right = right
	var strip := MeshInstance3D.new()
	strip.mesh = surface.commit()
	strip.material_override = material
	holder.add_child(strip)

func _chunk_centre_scene(chunk: Vector2i) -> Vector3:
	var origin: Vector3 = GenerationManager.chunk_coord_to_scene_origin(chunk, _anchor_chunk)
	return origin + Vector3(GenerationManager.BLOCKS_PER_CHUNK * 0.5, 0.0, GenerationManager.BLOCKS_PER_CHUNK * 0.5)

func _surface_at(x: float, z: float) -> float:
	return _world.sample_surface_height(int(floor(x)), int(floor(z)))

func _flat_material(colour: Color) -> StandardMaterial3D:
	if not _materials.has(colour):
		var material := StandardMaterial3D.new()
		material.albedo_color = colour
		material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		_materials[colour] = material
	return _materials[colour]
