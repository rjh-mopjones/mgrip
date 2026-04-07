extends Node

## World seed used by all generation calls.
var world_seed: int = 42

## Current chunk the player is in, in world-space integer coords.
var current_chunk: Vector2i = Vector2i.ZERO
