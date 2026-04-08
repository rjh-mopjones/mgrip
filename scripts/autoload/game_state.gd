extends Node

## World seed used by all generation calls.
var world_seed: int = 42

## Anchor runtime chunk coord used as the scene-space origin reference.
var anchor_chunk: Vector2i = Vector2i.ZERO

## Current runtime chunk coord occupied by the player.
var current_chunk: Vector2i = Vector2i.ZERO
