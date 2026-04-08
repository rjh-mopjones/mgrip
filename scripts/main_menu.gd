extends Control

const WORLD_SCENE_PATH := "res://scenes/world.tscn"
const MAP_SELECTOR_SCENE_PATH := "res://scenes/map_selector.tscn"
const WorldScript = preload("res://scripts/world/world.gd")

@onready var _quick_launch_button: Button = $CenterContainer/PanelContainer/MarginContainer/VBoxContainer/QuickLaunchButton
@onready var _open_map_button: Button = $CenterContainer/PanelContainer/MarginContainer/VBoxContainer/OpenMapButton

func _ready() -> void:
	var window := get_window()
	if window:
		window.size_changed.connect(_on_window_size_changed)
	_quick_launch_button.pressed.connect(_on_quick_launch_pressed)
	_open_map_button.pressed.connect(_on_open_map_pressed)

func _on_window_size_changed() -> void:
	var window := get_window()
	if window:
		print("main_menu window_size=", window.size)

func _on_quick_launch_pressed() -> void:
	GameState.set_direct_launch(_default_world_origin())
	get_tree().change_scene_to_file(WORLD_SCENE_PATH)

func _on_open_map_pressed() -> void:
	get_tree().change_scene_to_file(MAP_SELECTOR_SCENE_PATH)

func _default_world_origin() -> Vector2:
	var world_scene := load(WORLD_SCENE_PATH) as PackedScene
	if world_scene == null:
		return Vector2(WorldScript.DEFAULT_WORLD_X, WorldScript.DEFAULT_WORLD_Y)

	var world_root := world_scene.instantiate()
	var world_origin := Vector2(
		float(world_root.get("world_x")),
		float(world_root.get("world_y")),
	)
	world_root.free()
	return world_origin
