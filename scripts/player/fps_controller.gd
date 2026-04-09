extends CharacterBody3D

const SPEED            := 12.0   # m/s walk
const SPRINT_MULT      := 3.0
const JUMP_VELOCITY    := 10.0
const GRAVITY          := 25.0
const MOUSE_SENSITIVITY := 0.002
const PASSIVE_WINDOW_ARG := "--agent-runtime-passive-window"
const PASSIVE_WINDOW_ENV := "MG_AGENT_RUNTIME_PASSIVE_WINDOW"

const FLY_SPEED            := 12.0
const FLY_VERTICAL_SPEED   := 10.0
const SWIM_SPEED           := 6.0
const SWIM_VERTICAL_SPEED  := 5.0
const SWIM_GRAVITY         := 5.0
const SWIM_BUOYANCY        := 3.0

const DOUBLE_TAP_WINDOW := 0.3  # seconds

enum MoveState { WALKING, FLYING, SWIMMING }

@onready var _head: Node3D = $Head

var _scripted_motion_enabled := false
var _scripted_direction := Vector3.ZERO
var _scripted_speed := SPEED
var _scripted_fly_vertical := 0.0

var _move_state: MoveState = MoveState.WALKING
var _jumped := false
var _last_jump_press_msec := 0

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

func toggle_fly() -> void:
	if _move_state == MoveState.FLYING:
		_move_state = MoveState.WALKING
		_scripted_fly_vertical = 0.0
	else:
		_move_state = MoveState.FLYING
		velocity.y = 0.0

func clear_scripted_motion() -> void:
	_scripted_motion_enabled = false
	_scripted_direction = Vector3.ZERO
	_scripted_speed = SPEED
	_scripted_fly_vertical = 0.0
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

func _is_submerged() -> bool:
	var world := get_parent()
	if world == null or not world.has_method("get_chunk_streamer"):
		return false
	var streamer = world.get_chunk_streamer()
	if streamer == null:
		return false
	var anchor_chunk: Vector2i = GameState.anchor_chunk
	var chunk_coord := GenerationManager.scene_block_to_chunk_coord(
		anchor_chunk, position.x, position.z
	)
	var chunk = streamer.get_chunk(chunk_coord)
	if chunk == null or chunk.fluid_surface_mask.is_empty():
		return false
	var local := GenerationManager.scene_block_to_local_block(position.x, position.z)
	var idx := local.y * VoxelMeshBuilder.CHUNK_SIZE + local.x
	if idx < 0 or idx >= chunk.fluid_surface_mask.size():
		return false
	if not chunk.fluid_surface_mask[idx]:
		return false
	return position.y < float(VoxelMeshBuilder.SEA_LEVEL_Y)

func set_scripted_fly_vertical(value: float) -> void:
	_scripted_fly_vertical = clampf(value, -1.0, 1.0)

func _physics_process(delta: float) -> void:
	if not _scripted_motion_enabled and Input.is_action_just_pressed("jump"):
		var now := Time.get_ticks_msec()
		var is_double_tap := (now - _last_jump_press_msec) < int(DOUBLE_TAP_WINDOW * 1000)
		_last_jump_press_msec = now
		if is_double_tap:
			if _move_state == MoveState.FLYING:
				_move_state = MoveState.WALKING
				_scripted_fly_vertical = 0.0
			elif _move_state == MoveState.WALKING and not is_on_floor():
				_move_state = MoveState.FLYING
				velocity.y = 0.0

	if _move_state != MoveState.FLYING:
		if _is_submerged():
			if _move_state != MoveState.SWIMMING:
				_move_state = MoveState.SWIMMING
		elif _move_state == MoveState.SWIMMING:
			_move_state = MoveState.WALKING

	match _move_state:
		MoveState.WALKING:
			_process_walking(delta)
		MoveState.FLYING:
			_process_flying(delta)
		MoveState.SWIMMING:
			_process_swimming(delta)

	move_and_slide()

func _process_walking(delta: float) -> void:
	if is_on_floor():
		_jumped = false
	else:
		velocity.y -= GRAVITY * delta

	if Input.is_action_just_pressed("jump") and is_on_floor():
		velocity.y = JUMP_VELOCITY
		_jumped = true

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

func _process_flying(_delta: float) -> void:
	var vert := 0.0
	if not is_zero_approx(_scripted_fly_vertical):
		vert = _scripted_fly_vertical
	elif not _scripted_motion_enabled:
		if Input.is_action_pressed("jump"):
			vert += 1.0
		if Input.is_action_pressed("sprint"):
			vert -= 1.0
	velocity.y = vert * FLY_VERTICAL_SPEED

	if _scripted_motion_enabled:
		velocity.x = _scripted_direction.x * _scripted_speed
		velocity.z = _scripted_direction.z * _scripted_speed
		return
	var raw := Vector2(
		Input.get_axis("move_left", "move_right"),
		Input.get_axis("move_forward", "move_back"),
	)
	var dir := (transform.basis * Vector3(raw.x, 0.0, raw.y)).normalized()
	velocity.x = dir.x * FLY_SPEED if dir else move_toward(velocity.x, 0.0, FLY_SPEED)
	velocity.z = dir.z * FLY_SPEED if dir else move_toward(velocity.z, 0.0, FLY_SPEED)

func _process_swimming(delta: float) -> void:
	var water_surface_y := float(VoxelMeshBuilder.SEA_LEVEL_Y)
	var depth := water_surface_y - position.y

	if Input.is_action_pressed("jump"):
		velocity.y = SWIM_VERTICAL_SPEED
	elif Input.is_action_pressed("sprint"):
		velocity.y = -SWIM_VERTICAL_SPEED
	elif depth > 0.5:
		velocity.y -= SWIM_GRAVITY * delta
		velocity.y += SWIM_BUOYANCY * delta
	elif depth > -0.5:
		velocity.y = move_toward(velocity.y, 0.0, 8.0 * delta)
	else:
		velocity.y -= GRAVITY * delta

	if _scripted_motion_enabled:
		velocity.x = _scripted_direction.x * SWIM_SPEED
		velocity.z = _scripted_direction.z * SWIM_SPEED
		return
	var raw := Vector2(
		Input.get_axis("move_left", "move_right"),
		Input.get_axis("move_forward", "move_back"),
	)
	var dir := (transform.basis * Vector3(raw.x, 0.0, raw.y)).normalized()
	velocity.x = dir.x * SWIM_SPEED if dir else move_toward(velocity.x, 0.0, SWIM_SPEED)
	velocity.z = dir.z * SWIM_SPEED if dir else move_toward(velocity.z, 0.0, SWIM_SPEED)
