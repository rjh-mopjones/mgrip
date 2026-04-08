shader_type spatial;
render_mode cull_back, depth_draw_opaque, diffuse_burley, specular_schlick_ggx;

uniform float distance_haze_start = 700.0;
uniform float distance_haze_end = 2400.0;
uniform float lod_blend_strength = 0.0;
uniform vec3 horizon_tint : source_color = vec3(0.66, 0.41, 0.33);

varying vec3 world_pos;
varying vec3 local_normal;

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
}

void fragment() {
	vec3 base = COLOR.rgb;
	float top_mask = pow(clamp(local_normal.y, 0.0, 1.0), 2.8);
	float side_mask = 1.0 - top_mask;

	float broad = noise(world_pos.xz * 0.065);
	float grain = noise(world_pos.xz * 1.35 + vec2(world_pos.y * 0.08));
	float cliff_noise = noise(world_pos.zy * 0.22 + vec2(world_pos.x * 0.11, 0.0));
	float strata = 0.5 + 0.5 * sin(world_pos.y * 1.55 + cliff_noise * 5.0);

	float top_breakup = mix(broad, grain, 0.45);
	float cliff_breakup = mix(cliff_noise, strata, 0.55);
	top_breakup = smoothstep(0.18, 0.82, top_breakup);
	cliff_breakup = smoothstep(0.10, 0.90, cliff_breakup);

	vec3 top_dark = base * vec3(0.90, 0.88, 0.86);
	vec3 top_light = base * vec3(1.14, 1.08, 1.00);
	vec3 cliff_dark = base * vec3(0.42, 0.38, 0.38);
	vec3 cliff_light = base * vec3(0.72, 0.62, 0.58);
	vec3 top_albedo = mix(top_dark, top_light, top_breakup);
	vec3 cliff_albedo = mix(cliff_dark, cliff_light, cliff_breakup);
	cliff_albedo = mix(cliff_albedo, vec3(0.30, 0.18, 0.16), side_mask * 0.28);
	vec3 albedo = mix(cliff_albedo, top_albedo, top_mask);

	float dust = noise(world_pos.xz * 0.18 + vec2(41.0, 17.0)) * top_mask;
	albedo = mix(albedo, vec3(0.86, 0.62, 0.41), dust * 0.14);
	albedo *= mix(0.82, 1.05, top_mask);
	albedo *= mix(0.72, 1.0, top_mask);
	albedo = mix(albedo, albedo * 0.78, side_mask * 0.45);

	float view_distance = distance(CAMERA_POSITION_WORLD, world_pos);
	float haze = smoothstep(distance_haze_start, distance_haze_end, view_distance);
	float lod_haze = clamp(haze * (0.45 + lod_blend_strength), 0.0, 1.0);
	albedo = mix(albedo, horizon_tint, lod_haze * 0.34);
	albedo = mix(albedo, albedo * 0.92 + horizon_tint * 0.08, lod_blend_strength * 0.16);

	ALBEDO = clamp(albedo, vec3(0.0), vec3(1.0));
	ROUGHNESS = mix(1.0, 0.86, top_breakup * 0.35 + top_mask * 0.20);
	ROUGHNESS = mix(ROUGHNESS, 1.0, lod_haze * 0.18);
	METALLIC = 0.0;
	SPECULAR = 0.12;
}
