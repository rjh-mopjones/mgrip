shader_type spatial;
render_mode cull_back, depth_draw_opaque, diffuse_burley, specular_schlick_ggx;

uniform float distance_haze_start = 700.0;
uniform float distance_haze_end = 2400.0;
uniform float lod_blend_strength = 0.0;
uniform vec3 horizon_tint : source_color = vec3(0.66, 0.41, 0.33);
uniform vec3 palette_top_tint : source_color = vec3(0.60, 0.45, 0.38);
uniform vec3 palette_cliff_tint : source_color = vec3(0.34, 0.28, 0.26);
uniform vec3 palette_shadow_tint : source_color = vec3(0.20, 0.14, 0.14);
uniform float palette_top_strength = 0.24;
uniform float palette_cliff_strength = 0.30;
uniform float palette_shadow_strength = 0.18;
uniform float palette_dust_strength = 0.10;
uniform float palette_darkness = 0.08;
uniform float palette_coldness = 0.0;
uniform float average_snowpack = 0.0;
uniform float average_water_table = 0.0;
uniform float average_aridity = 0.5;
uniform float landform_top_variation = 0.42;
uniform float landform_cliff_variation = 0.38;
uniform float landform_strata_strength = 0.32;
uniform float landform_cliff_boost = 0.12;
uniform float landform_shadow_boost = 0.10;
uniform float landform_roughness_boost = 0.04;
uniform float interestingness_boost = 0.0;
uniform bool use_reduced_presentation_textures = false;
uniform sampler2D landform_grid_texture : source_color;
uniform sampler2D palette_weight_grid_texture : source_color;
uniform sampler2D palette_tint_grid_texture : source_color;

varying vec3 world_pos;
varying vec3 local_normal;
varying vec3 local_pos;

float hash(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);

	return mix(
		mix(hash(i + vec2(0.0, 0.0)), hash(i + vec2(1.0, 0.0)), u.x),
		mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
		u.y
	);
}

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	local_normal = normalize(NORMAL);
	local_pos = VERTEX;
}

void fragment() {
	vec2 presentation_uv = clamp(local_pos.xz / 512.0, vec2(0.0), vec2(1.0));
	vec4 local_landform_sample = vec4(
		landform_top_variation,
		landform_cliff_variation,
		landform_strata_strength,
		clamp((landform_cliff_boost + landform_shadow_boost) * 2.0, 0.0, 1.0)
	);
	vec4 local_palette_weight_sample = vec4(
		palette_top_strength,
		palette_cliff_strength,
		palette_dust_strength,
		clamp(palette_darkness * 3.0, 0.0, 1.0)
	);
	vec4 local_palette_tint_sample = vec4(
		mix(palette_top_tint, palette_cliff_tint, 0.35),
		palette_shadow_strength
	);
	if (use_reduced_presentation_textures) {
		local_landform_sample = texture(landform_grid_texture, presentation_uv);
		local_palette_weight_sample = texture(palette_weight_grid_texture, presentation_uv);
		local_palette_tint_sample = texture(palette_tint_grid_texture, presentation_uv);
	}

	float local_top_variation = mix(landform_top_variation, local_landform_sample.r, 0.65);
	float local_cliff_variation = mix(landform_cliff_variation, local_landform_sample.g, 0.65);
	float local_strata_strength = mix(landform_strata_strength, local_landform_sample.b, 0.65);
	float local_accent_boost = local_landform_sample.a * 0.16;
	float local_cliff_boost = mix(landform_cliff_boost, local_accent_boost, 0.55);
	float local_shadow_boost = mix(landform_shadow_boost, local_accent_boost, 0.45);
	float local_roughness_boost = mix(
		landform_roughness_boost,
		local_landform_sample.g * 0.10,
		0.45
	);
	float local_palette_top_strength = mix(
		palette_top_strength,
		local_palette_weight_sample.r,
		0.70
	);
	float local_palette_cliff_strength = mix(
		palette_cliff_strength,
		local_palette_weight_sample.g,
		0.70
	);
	float local_palette_dust_strength = mix(
		palette_dust_strength,
		local_palette_weight_sample.b,
		0.70
	);
	float local_palette_darkness = mix(
		palette_darkness,
		local_palette_weight_sample.a / 3.0,
		0.70
	);
	float local_palette_shadow_strength = mix(
		palette_shadow_strength,
		local_palette_tint_sample.a,
		0.60
	);
	vec3 local_palette_tint = mix(palette_top_tint, local_palette_tint_sample.rgb, 0.60);
	vec3 face_normal = normalize(cross(dFdx(world_pos), dFdy(world_pos)));
	if (dot(face_normal, local_normal) < 0.0) {
		face_normal *= -1.0;
	}
	float facet_strength = clamp(
		0.52 + local_cliff_boost * 1.6 + local_shadow_boost * 0.8 + interestingness_boost * 0.18,
		0.52,
		0.92
	);
	vec3 terrain_normal = normalize(mix(local_normal, face_normal, facet_strength));
	float ridge_light = clamp(dot(terrain_normal, normalize(vec3(0.34, 0.88, 0.26))), 0.0, 1.0);
	float crevice_light = clamp(dot(terrain_normal, normalize(vec3(-0.48, 0.56, -0.68))), 0.0, 1.0);

	float base_luma = dot(COLOR.rgb, vec3(0.299, 0.587, 0.114));
	vec3 base = mix(vec3(base_luma), COLOR.rgb, 0.10);
	float top_mask = pow(clamp(terrain_normal.y, 0.0, 1.0), 3.6);
	float side_mask = 1.0 - top_mask;

	float broad_scale = mix(0.040, 0.095, local_top_variation);
	float grain_scale = mix(0.90, 2.10, local_top_variation);
	float cliff_scale = mix(0.18, 0.42, local_cliff_variation);
	float strata_scale = mix(1.15, 2.80, local_strata_strength);
	float broad = noise(world_pos.xz * broad_scale);
	float grain = noise(world_pos.xz * grain_scale + vec2(world_pos.y * 0.08));
	float cliff_noise = noise(world_pos.zy * cliff_scale + vec2(world_pos.x * 0.11, 0.0));
	float strata = 0.5 + 0.5 * sin(world_pos.y * strata_scale + cliff_noise * mix(3.0, 6.8, local_strata_strength));
	float terrace_frequency = mix(0.08, 0.22, local_strata_strength);
	float terrace_phase = abs(fract(
		world_pos.y * terrace_frequency + cliff_noise * (0.35 + local_cliff_variation * 0.25)
	) - 0.5) * 2.0;
	float terrace_mask = 1.0 - smoothstep(0.18, 0.72, terrace_phase);

	float top_breakup = mix(broad, grain, 0.45);
	float cliff_breakup = mix(cliff_noise, strata, 0.55);
	top_breakup = smoothstep(
		max(0.05, 0.18 - local_top_variation * 0.10),
		min(0.95, 0.82 + interestingness_boost * 0.08),
		top_breakup
	);
	cliff_breakup = smoothstep(
		max(0.02, 0.10 - local_cliff_variation * 0.04),
		min(0.98, 0.90 + local_cliff_variation * 0.06),
		cliff_breakup
	);

	vec3 top_dark = base * vec3(0.90, 0.88, 0.86);
	vec3 top_light = base * vec3(1.14, 1.08, 1.00);
	vec3 cliff_dark = base * vec3(0.42, 0.38, 0.38);
	vec3 cliff_light = base * vec3(0.72, 0.62, 0.58);
	vec3 top_albedo = mix(top_dark, top_light, top_breakup);
	vec3 cliff_albedo = mix(cliff_dark, cliff_light, cliff_breakup);
	top_albedo = mix(
		top_albedo,
		palette_top_tint,
		clamp(0.24 + local_palette_top_strength * 0.76, 0.0, 1.0)
	);
	top_albedo = mix(
		top_albedo,
		local_palette_tint,
		clamp(0.10 + local_palette_top_strength * 0.36, 0.0, 1.0)
	);
	cliff_albedo = mix(
		cliff_albedo,
		palette_cliff_tint,
		clamp(0.22 + local_palette_cliff_strength * 0.78, 0.0, 1.0)
	);
	cliff_albedo = mix(
		cliff_albedo,
		local_palette_tint,
		clamp(0.08 + local_palette_cliff_strength * 0.30, 0.0, 1.0)
	);
	cliff_albedo = mix(
		cliff_albedo,
		palette_shadow_tint,
		clamp(side_mask * 0.28 + local_palette_shadow_strength * 0.25, 0.0, 1.0)
	);
	vec3 albedo = mix(cliff_albedo, top_albedo, top_mask);

	float dust = noise(world_pos.xz * 0.18 + vec2(41.0, 17.0)) * top_mask;
	vec3 dust_tint = mix(palette_top_tint, local_palette_tint, 0.55);
	albedo = mix(
		albedo,
		dust_tint,
		dust * local_palette_dust_strength * mix(0.45, 1.0, average_aridity)
	);
	albedo *= mix(0.72, 1.10, ridge_light);
	albedo *= mix(0.68, 1.02, top_mask);
	albedo = mix(albedo, albedo * 0.70, side_mask * (0.22 + local_cliff_boost * 0.42));
	albedo = mix(
		albedo,
		albedo * 0.76,
		(1.0 - crevice_light) * side_mask * (0.14 + local_shadow_boost * 0.28)
	);
	albedo = mix(
		albedo,
		albedo * 0.82,
		terrace_mask * (0.08 + local_strata_strength * 0.18) * (0.35 + side_mask * 0.65)
	);
	albedo = mix(albedo, albedo * 1.05, terrace_mask * top_mask * 0.06);

	float wetness = smoothstep(0.18, 0.80, average_water_table) * (1.0 - average_snowpack * 0.55);
	vec3 wet_albedo = mix(albedo * 0.78, palette_cliff_tint * 0.88, 0.35);
	albedo = mix(albedo, wet_albedo, wetness * (0.12 + side_mask * 0.18));

	float snow_overlay = smoothstep(0.18, 0.78, average_snowpack) * top_mask;
	vec3 snow_tint = mix(vec3(0.82, 0.86, 0.90), vec3(0.70, 0.82, 0.96), palette_coldness);
	albedo = mix(albedo, snow_tint, snow_overlay * (0.35 + palette_coldness * 0.35));

	albedo = mix(albedo, palette_shadow_tint, local_palette_shadow_strength * (0.12 + side_mask * 0.16));
	albedo = mix(
		albedo,
		palette_shadow_tint,
		(side_mask * local_cliff_boost * 0.20) + (local_shadow_boost * 0.10)
	);
	albedo *= mix(1.0, 0.84, local_palette_darkness);

	float green_cap = max(albedo.r * 0.96, albedo.b * 0.98 + 0.02);
	albedo.g = min(albedo.g, green_cap);

	float view_distance = distance(CAMERA_POSITION_WORLD, world_pos);
	float haze = smoothstep(distance_haze_start, distance_haze_end, view_distance);
	float lod_haze = clamp(haze * (0.45 + lod_blend_strength), 0.0, 1.0);
	albedo = mix(albedo, horizon_tint, lod_haze * 0.20);
	albedo = mix(albedo, albedo * 0.96 + horizon_tint * 0.04, lod_blend_strength * 0.10);

	ALBEDO = clamp(albedo, vec3(0.0), vec3(1.0));
	ROUGHNESS = mix(
		0.98,
		0.88 - local_roughness_boost * 0.10,
		top_breakup * (0.28 + interestingness_boost * 0.10) + top_mask * 0.12
	);
	ROUGHNESS = clamp(
		ROUGHNESS
		+ 0.08
		+ (1.0 - ridge_light) * 0.06
		+ side_mask * local_cliff_boost * 0.16
		+ terrace_mask * 0.04,
		0.0,
		1.0
	);
	ROUGHNESS = mix(ROUGHNESS, 1.0, lod_haze * 0.18);
	METALLIC = 0.0;
	SPECULAR = 0.02;
}
