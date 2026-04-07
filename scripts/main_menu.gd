extends Node

func _ready() -> void:
	DisplayServer.window_set_size(Vector2i(1600, 900))
	get_tree().call_deferred("change_scene_to_file", "res://scenes/world.tscn")
