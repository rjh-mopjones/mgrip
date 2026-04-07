extends CharacterBody3D

const SPEED            := 12.0   # m/s walk
const SPRINT_MULT      := 3.0
const JUMP_VELOCITY    := 10.0
const GRAVITY          := 25.0
const MOUSE_SENSITIVITY := 0.002

@onready var _head: Node3D = $Head

func _ready() -> void:
	get_window().grab_focus()
	Input.set_mouse_mode(Input.MOUSE_MODE_CAPTURED)

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseMotion and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
		rotate_y(-event.relative.x * MOUSE_SENSITIVITY)
		_head.rotate_x(-event.relative.y * MOUSE_SENSITIVITY)
		_head.rotation.x = clampf(_head.rotation.x, -PI * 0.45, PI * 0.45)
	if event.is_action_pressed("ui_cancel"):
		Input.set_mouse_mode(Input.MOUSE_MODE_VISIBLE)
	elif event is InputEventMouseButton and event.pressed:
		Input.set_mouse_mode(Input.MOUSE_MODE_CAPTURED)

func _physics_process(delta: float) -> void:
	if not is_on_floor():
		velocity.y -= GRAVITY * delta

	if Input.is_action_just_pressed("jump") and is_on_floor():
		velocity.y = JUMP_VELOCITY

	var speed := SPEED * (SPRINT_MULT if Input.is_action_pressed("sprint") else 1.0)
	var raw := Vector2(
		Input.get_axis("move_left", "move_right"),
		Input.get_axis("move_forward", "move_back"),
	)
	var dir := (transform.basis * Vector3(raw.x, 0.0, raw.y)).normalized()
	velocity.x = dir.x * speed if dir else move_toward(velocity.x, 0.0, speed)
	velocity.z = dir.z * speed if dir else move_toward(velocity.z, 0.0, speed)

	move_and_slide()
