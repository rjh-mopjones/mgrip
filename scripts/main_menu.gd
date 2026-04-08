extends Control

const WORLD_SCENE_PATH := "res://scenes/world.tscn"
const MAP_SELECTOR_SCENE_PATH := "res://scenes/map_selector.tscn"
const WorldScript = preload("res://scripts/world/world.gd")
const AGENT_RUNTIME_QUICK_LAUNCH_ARG := "--agent-runtime-quick-launch"
const AGENT_RUNTIME_SMOKE_ARG := "--agent-runtime-smoke-test"
const AGENT_RUNTIME_WORLD_ORIGIN_ENV := "MG_AGENT_RUNTIME_WORLD_ORIGIN"
const AGENT_RUNTIME_WORLD_ORIGIN_ARG_PREFIX := "--agent-runtime-world-origin="

@onready var _quick_launch_button: Button = $CenterContainer/PanelContainer/MarginContainer/VBoxContainer/QuickLaunchButton
@onready var _open_map_button: Button = $CenterContainer/PanelContainer/MarginContainer/VBoxContainer/OpenMapButton

func _ready() -> void:
	var window := get_window()
	if window:
		window.size_changed.connect(_on_window_size_changed)
	_quick_launch_button.pressed.connect(_on_quick_launch_pressed)
	_open_map_button.pressed.connect(_on_open_map_pressed)
	if _wants_agent_runtime_quick_launch():
		print("main_menu: auto quick launch for agent runtime")
		call_deferred("_on_quick_launch_pressed")

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
	var runtime_override: Variant = _agent_runtime_world_origin_override()
	if runtime_override != null:
		return runtime_override as Vector2
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

func _wants_agent_runtime_quick_launch() -> bool:
	var args: Array = []
	args.append_array(OS.get_cmdline_args())
	args.append_array(OS.get_cmdline_user_args())
	return AGENT_RUNTIME_QUICK_LAUNCH_ARG in args or AGENT_RUNTIME_SMOKE_ARG in args

func _agent_runtime_world_origin_override():
	var env_value := OS.get_environment(AGENT_RUNTIME_WORLD_ORIGIN_ENV).strip_edges()
	if not env_value.is_empty():
		var parsed_env: Variant = _parse_world_origin_override(env_value)
		if parsed_env != null:
			return parsed_env
	var args: Array = []
	args.append_array(OS.get_cmdline_args())
	args.append_array(OS.get_cmdline_user_args())
	for arg in args:
		var value := String(arg)
		if value.begins_with(AGENT_RUNTIME_WORLD_ORIGIN_ARG_PREFIX):
			var parsed_arg: Variant = _parse_world_origin_override(
				value.trim_prefix(AGENT_RUNTIME_WORLD_ORIGIN_ARG_PREFIX)
			)
			if parsed_arg != null:
				return parsed_arg
	return null

func _parse_world_origin_override(value: String):
	var parts := value.split(",", false)
	if parts.size() != 2:
		push_warning("main_menu: invalid agent runtime world origin override '%s'" % value)
		return null
	if not parts[0].is_valid_float() or not parts[1].is_valid_float():
		push_warning("main_menu: invalid numeric world origin override '%s'" % value)
		return null
	return Vector2(parts[0].to_float(), parts[1].to_float())
