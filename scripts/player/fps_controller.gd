extends CharacterBody3D

const SPEED            := 12.0   # m/s walk
const SPRINT_MULT      := 3.0
const JUMP_VELOCITY    := 10.0
const GRAVITY          := 25.0
const MOUSE_SENSITIVITY := 0.002
const PASSIVE_WINDOW_ARG := "--agent-runtime-passive-window"
const PASSIVE_WINDOW_ENV := "MG_AGENT_RUNTIME_PASSIVE_WINDOW"

@onready var _head: Node3D = $Head

var _scripted_motion_enabled := false
var _scripted_direction := Vector3.ZERO
var _scripted_speed := SPEED

func _ready() -> void:
	if _is_passive_agent_runtime_window():
		Input.set_mouse_mode(Input.MOUSE_MODE_VISIBLE)
		return
	get_window().grab_focus()
	Input.set_mouse_mode(Input.MOUSE_MODE_CAPTURED)

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseMotion and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
		rotate_y(-event.relative.x * MOUSE_SENSITIVITY)
		_head.rotate_x(-event.relative.y * MOUSE_SENSITIVITY)
		_head.rotation.x = clampf(_head.rotation.x, -PI * 0.45, PI * 0.45)
	if _is_passive_agent_runtime_window():
		return
	if event.is_action_pressed("ui_cancel"):
		Input.set_mouse_mode(Input.MOUSE_MODE_VISIBLE)
	elif event is InputEventMouseButton and event.pressed:
		Input.set_mouse_mode(Input.MOUSE_MODE_CAPTURED)

func set_scripted_motion(direction: Vector3, speed: float = SPEED) -> void:
	_scripted_direction = Vector3(direction.x, 0.0, direction.z).normalized()
	_scripted_speed = speed
	_scripted_motion_enabled = not _scripted_direction.is_zero_approx()

func set_scripted_look(yaw_degrees: float, pitch_degrees: float) -> void:
	rotation_degrees.y = yaw_degrees
	_head.rotation_degrees.x = clampf(pitch_degrees, -rad_to_deg(PI * 0.45), rad_to_deg(PI * 0.45))

func set_scripted_look_at(target_point: Vector3) -> void:
	var from_origin := _head.global_transform.origin
	var direction := target_point - from_origin
	if direction.length_squared() <= 0.0001:
		return
	var planar := Vector2(direction.x, direction.z)
	var yaw := rotation_degrees.y
	if planar.length_squared() > 0.0001:
		yaw = rad_to_deg(atan2(-direction.x, -direction.z))
	var pitch := rad_to_deg(asin(clampf(direction.normalized().y, -1.0, 1.0)))
	set_scripted_look(yaw, pitch)

func current_facing_direction() -> Vector3:
	return -_head.global_transform.basis.z

func is_scripted_motion_active() -> bool:
	return _scripted_motion_enabled

func clear_scripted_motion() -> void:
	_scripted_motion_enabled = false
	_scripted_direction = Vector3.ZERO
	_scripted_speed = SPEED
	velocity.x = 0.0
	velocity.z = 0.0

func _is_passive_agent_runtime_window() -> bool:
	var env_value := OS.get_environment(PASSIVE_WINDOW_ENV).to_lower()
	if env_value == "1" or env_value == "true" or env_value == "yes":
		return true
	var args := PackedStringArray()
	args.append_array(OS.get_cmdline_args())
	args.append_array(OS.get_cmdline_user_args())
	return PASSIVE_WINDOW_ARG in args

func _physics_process(delta: float) -> void:
	if not is_on_floor():
		velocity.y -= GRAVITY * delta

	if Input.is_action_just_pressed("jump") and is_on_floor():
		velocity.y = JUMP_VELOCITY

	var speed := _scripted_speed if _scripted_motion_enabled else SPEED * (SPRINT_MULT if Input.is_action_pressed("sprint") else 1.0)
	var dir := _scripted_direction if _scripted_motion_enabled else Vector3.ZERO
	if not _scripted_motion_enabled:
		var raw := Vector2(
			Input.get_axis("move_left", "move_right"),
			Input.get_axis("move_forward", "move_back"),
		)
		dir = (transform.basis * Vector3(raw.x, 0.0, raw.y)).normalized()
	velocity.x = dir.x * speed if dir else move_toward(velocity.x, 0.0, speed)
	velocity.z = dir.z * speed if dir else move_toward(velocity.z, 0.0, speed)

	move_and_slide()
