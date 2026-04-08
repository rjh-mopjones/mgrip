extends RefCounted
class_name WorldEnvironmentController

const SKY_SHADER := preload("res://assets/shaders/sky.gdshader")

var _world_environment: WorldEnvironment
var _environment: Environment
var _sky_material: ShaderMaterial
var _sun: DirectionalLight3D

func setup(world_environment: WorldEnvironment, sun: DirectionalLight3D = null) -> void:
	_world_environment = world_environment
	_sun = sun
	if _world_environment == null:
		return
	_environment = _prepare_environment(_world_environment.environment)
	_world_environment.environment = _environment
	_sky_material = _prepare_sky_material(_environment)

func apply_runtime_presentation(runtime_presentation: Dictionary) -> void:
	if _environment == null or runtime_presentation.is_empty():
		return
	var atmosphere_name := _enum_name(runtime_presentation.get("atmosphere_class", {}), "TemperateTwilight")
	var zone_name := _enum_name(runtime_presentation.get("planet_zone", {}), "InnerTerminus")
	var profile := _profile_for(atmosphere_name, zone_name)
	_environment.ambient_light_color = profile["ambient_color"]
	_environment.ambient_light_energy = float(profile["ambient_energy"])
	_environment.fog_enabled = true
	_environment.fog_light_color = profile["fog_color"]
	_environment.fog_density = float(profile["fog_density"])
	if _sky_material != null:
		_sky_material.set_shader_parameter("zenith_color", profile["zenith_color"])
		_sky_material.set_shader_parameter("upper_color", profile["upper_color"])
		_sky_material.set_shader_parameter("horizon_color", profile["horizon_color"])
		_sky_material.set_shader_parameter("haze_color", profile["haze_color"])
		_sky_material.set_shader_parameter("horizon_height", float(profile["horizon_height"]))
		_sky_material.set_shader_parameter("horizon_softness", float(profile["horizon_softness"]))
		_sky_material.set_shader_parameter("sun_disc_size", float(profile["sun_disc_size"]))
		_sky_material.set_shader_parameter("sun_glow_size", float(profile["sun_glow_size"]))
	if _sun != null:
		_sun.light_color = profile["sun_color"]
		_sun.light_energy = float(profile["sun_energy"])

func _prepare_environment(base_environment: Environment) -> Environment:
	if base_environment == null:
		var environment := Environment.new()
		environment.background_mode = Environment.BG_SKY
		return environment
	return base_environment.duplicate(true)

func _prepare_sky_material(environment: Environment) -> ShaderMaterial:
	var sky := environment.sky
	if sky == null:
		sky = Sky.new()
	else:
		sky = sky.duplicate(true)
	environment.sky = sky
	var material = sky.sky_material as ShaderMaterial
	if material == null:
		material = ShaderMaterial.new()
		material.shader = SKY_SHADER
	else:
		material = material.duplicate(true)
	sky.sky_material = material
	return material

func _profile_for(atmosphere_name: String, zone_name: String) -> Dictionary:
	var profile := {
		"zenith_color": Color(0.04, 0.02, 0.07),
		"upper_color": Color(0.16, 0.05, 0.08),
		"horizon_color": Color(0.52, 0.18, 0.10),
		"haze_color": Color(0.92, 0.42, 0.16),
		"ambient_color": Color(0.15, 0.10, 0.12),
		"ambient_energy": 1.45,
		"fog_color": Color(0.58, 0.18, 0.10),
		"fog_density": 0.00018,
		"sun_color": Color(0.98, 0.74, 0.52),
		"sun_energy": 1.05,
		"horizon_height": -0.08,
		"horizon_softness": 0.30,
		"sun_disc_size": 0.045,
		"sun_glow_size": 0.22,
	}
	match atmosphere_name:
		"BlastedRadiance":
			profile = {
				"zenith_color": Color(0.08, 0.02, 0.02),
				"upper_color": Color(0.28, 0.06, 0.03),
				"horizon_color": Color(0.88, 0.26, 0.10),
				"haze_color": Color(1.00, 0.58, 0.18),
				"ambient_color": Color(0.28, 0.12, 0.08),
				"ambient_energy": 1.85,
				"fog_color": Color(0.88, 0.32, 0.12),
				"fog_density": 0.00022,
				"sun_color": Color(1.00, 0.72, 0.40),
				"sun_energy": 1.30,
				"horizon_height": -0.10,
				"horizon_softness": 0.24,
				"sun_disc_size": 0.050,
				"sun_glow_size": 0.25,
			}
		"HarshAmberHaze":
			profile = {
				"zenith_color": Color(0.06, 0.02, 0.03),
				"upper_color": Color(0.22, 0.06, 0.05),
				"horizon_color": Color(0.78, 0.26, 0.11),
				"haze_color": Color(0.98, 0.48, 0.16),
				"ambient_color": Color(0.21, 0.11, 0.10),
				"ambient_energy": 1.62,
				"fog_color": Color(0.72, 0.24, 0.12),
				"fog_density": 0.00020,
				"sun_color": Color(0.99, 0.73, 0.49),
				"sun_energy": 1.16,
				"horizon_height": -0.08,
				"horizon_softness": 0.27,
				"sun_disc_size": 0.046,
				"sun_glow_size": 0.23,
			}
		"DryTwilight":
			profile = {
				"zenith_color": Color(0.03, 0.02, 0.08),
				"upper_color": Color(0.11, 0.04, 0.09),
				"horizon_color": Color(0.44, 0.19, 0.16),
				"haze_color": Color(0.74, 0.34, 0.18),
				"ambient_color": Color(0.13, 0.10, 0.15),
				"ambient_energy": 1.32,
				"fog_color": Color(0.44, 0.20, 0.16),
				"fog_density": 0.00016,
				"sun_color": Color(0.95, 0.70, 0.56),
				"sun_energy": 0.92,
				"horizon_height": -0.06,
				"horizon_softness": 0.34,
				"sun_disc_size": 0.040,
				"sun_glow_size": 0.20,
			}
		"TemperateTwilight":
			profile = {
				"zenith_color": Color(0.02, 0.02, 0.08),
				"upper_color": Color(0.08, 0.04, 0.10),
				"horizon_color": Color(0.20, 0.14, 0.18),
				"haze_color": Color(0.44, 0.28, 0.30),
				"ambient_color": Color(0.11, 0.10, 0.15),
				"ambient_energy": 1.26,
				"fog_color": Color(0.24, 0.18, 0.22),
				"fog_density": 0.00018,
				"sun_color": Color(0.86, 0.68, 0.66),
				"sun_energy": 0.86,
				"horizon_height": -0.05,
				"horizon_softness": 0.36,
				"sun_disc_size": 0.038,
				"sun_glow_size": 0.18,
			}
		"WetTwilight":
			profile = {
				"zenith_color": Color(0.02, 0.03, 0.09),
				"upper_color": Color(0.07, 0.05, 0.10),
				"horizon_color": Color(0.28, 0.18, 0.18),
				"haze_color": Color(0.54, 0.28, 0.22),
				"ambient_color": Color(0.10, 0.12, 0.16),
				"ambient_energy": 1.28,
				"fog_color": Color(0.28, 0.20, 0.20),
				"fog_density": 0.00024,
				"sun_color": Color(0.86, 0.67, 0.64),
				"sun_energy": 0.82,
				"horizon_height": -0.04,
				"horizon_softness": 0.38,
				"sun_disc_size": 0.038,
				"sun_glow_size": 0.18,
			}
		"FrostTwilight":
			profile = {
				"zenith_color": Color(0.01, 0.03, 0.08),
				"upper_color": Color(0.05, 0.07, 0.12),
				"horizon_color": Color(0.22, 0.22, 0.26),
				"haze_color": Color(0.58, 0.62, 0.72),
				"ambient_color": Color(0.10, 0.13, 0.18),
				"ambient_energy": 1.10,
				"fog_color": Color(0.48, 0.54, 0.65),
				"fog_density": 0.00020,
				"sun_color": Color(0.82, 0.86, 0.96),
				"sun_energy": 0.72,
				"horizon_height": -0.03,
				"horizon_softness": 0.40,
				"sun_disc_size": 0.036,
				"sun_glow_size": 0.17,
			}
		"PolarGlow":
			profile = {
				"zenith_color": Color(0.00, 0.02, 0.06),
				"upper_color": Color(0.03, 0.06, 0.12),
				"horizon_color": Color(0.16, 0.18, 0.22),
				"haze_color": Color(0.48, 0.70, 0.80),
				"ambient_color": Color(0.07, 0.10, 0.16),
				"ambient_energy": 0.88,
				"fog_color": Color(0.34, 0.44, 0.58),
				"fog_density": 0.00018,
				"sun_color": Color(0.62, 0.78, 0.92),
				"sun_energy": 0.42,
				"horizon_height": -0.02,
				"horizon_softness": 0.42,
				"sun_disc_size": 0.032,
				"sun_glow_size": 0.15,
			}
		"BlackIceDark":
			profile = {
				"zenith_color": Color(0.00, 0.01, 0.04),
				"upper_color": Color(0.01, 0.03, 0.06),
				"horizon_color": Color(0.04, 0.06, 0.10),
				"haze_color": Color(0.18, 0.28, 0.36),
				"ambient_color": Color(0.04, 0.06, 0.11),
				"ambient_energy": 0.58,
				"fog_color": Color(0.10, 0.16, 0.22),
				"fog_density": 0.00014,
				"sun_color": Color(0.36, 0.48, 0.62),
				"sun_energy": 0.20,
				"horizon_height": -0.01,
				"horizon_softness": 0.45,
				"sun_disc_size": 0.028,
				"sun_glow_size": 0.12,
			}
		"GeothermalNight":
			profile = {
				"zenith_color": Color(0.02, 0.01, 0.03),
				"upper_color": Color(0.08, 0.02, 0.04),
				"horizon_color": Color(0.26, 0.10, 0.08),
				"haze_color": Color(0.74, 0.30, 0.14),
				"ambient_color": Color(0.11, 0.06, 0.09),
				"ambient_energy": 0.82,
				"fog_color": Color(0.36, 0.14, 0.10),
				"fog_density": 0.00018,
				"sun_color": Color(0.86, 0.42, 0.20),
				"sun_energy": 0.34,
				"horizon_height": -0.03,
				"horizon_softness": 0.38,
				"sun_disc_size": 0.034,
				"sun_glow_size": 0.18,
			}
	if zone_name == "AbyssalNight":
		profile["ambient_energy"] = maxf(0.42, float(profile["ambient_energy"]) * 0.82)
		profile["sun_energy"] = maxf(0.12, float(profile["sun_energy"]) * 0.70)
		profile["fog_density"] = float(profile["fog_density"]) * 0.85
	elif zone_name == "SubstellarInferno":
		profile["ambient_energy"] = float(profile["ambient_energy"]) * 1.08
		profile["sun_energy"] = float(profile["sun_energy"]) * 1.08
		profile["fog_density"] = float(profile["fog_density"]) * 1.08
	return profile

func _enum_name(value, fallback: String) -> String:
	if value is Dictionary:
		return String((value as Dictionary).get("name", fallback))
	return fallback
