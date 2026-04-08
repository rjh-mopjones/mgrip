extends Node

enum LaunchMode {
	DIRECT_COORD,
	SELECTED_CHUNK,
}

## World seed used by all generation calls.
var world_seed: int = 42

## Anchor runtime chunk coord used as the scene-space origin reference.
var anchor_chunk: Vector2i = Vector2i.ZERO

## Current runtime chunk coord occupied by the player.
var current_chunk: Vector2i = Vector2i.ZERO

## Pending pre-world launch request shared by menu flows.
var launch_mode: LaunchMode = LaunchMode.DIRECT_COORD
var launch_world_origin: Vector2 = Vector2.ZERO
var launch_chunk: Vector2i = Vector2i.ZERO
var has_pending_launch: bool = false

func set_direct_launch(world_origin: Vector2) -> void:
	launch_mode = LaunchMode.DIRECT_COORD
	launch_world_origin = world_origin
	launch_chunk = Vector2i.ZERO
	has_pending_launch = true

func set_selected_chunk_launch(chunk_coord: Vector2i) -> void:
	launch_mode = LaunchMode.SELECTED_CHUNK
	launch_chunk = chunk_coord
	launch_world_origin = Vector2.ZERO
	has_pending_launch = true

func clear_launch_request() -> void:
	has_pending_launch = false
	launch_mode = LaunchMode.DIRECT_COORD
	launch_world_origin = Vector2.ZERO
	launch_chunk = Vector2i.ZERO
