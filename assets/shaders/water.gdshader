shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_disabled, diffuse_burley, specular_schlick_ggx;

uniform vec3  shallow_color : source_color = vec3(0.16, 0.20, 0.42);
uniform vec3  deep_color    : source_color = vec3(0.04, 0.05, 0.16);
uniform float wave_speed    = 0.8;
uniform float wave_height   = 0.15;
uniform float alpha_min     = 0.45;
uniform float alpha_max     = 0.88;
uniform float roughness_value = 0.15;
uniform float specular_value = 0.5;
uniform float metallic_value = 0.1;
uniform bool use_reduced_water_grid = false;
uniform sampler2D water_state_grid_texture : source_color;

varying float local_frozen;
varying float local_mineral;
varying float local_liquidness;
varying float local_edge;

void vertex() {
	vec2 presentation_uv = clamp(VERTEX.xz / 512.0, vec2(0.0), vec2(1.0));
	vec4 local_profile = vec4(0.0, 0.04, 1.0, 0.14);
	if (use_reduced_water_grid) {
		local_profile = texture(water_state_grid_texture, presentation_uv);
	}
	local_frozen = local_profile.r;
	local_mineral = local_profile.g;
	local_liquidness = local_profile.b;
	local_edge = local_profile.a;
	float dynamic_wave_height = wave_height * mix(0.08, 1.0, local_liquidness) * (1.0 - local_frozen * 0.85);
	float dynamic_wave_speed = wave_speed * mix(0.18, 1.0, local_liquidness);
	float wave1 = sin(VERTEX.x * 0.5 + TIME * dynamic_wave_speed) * dynamic_wave_height;
	float wave2 = sin(VERTEX.z * 0.7 + TIME * dynamic_wave_speed * 0.8) * dynamic_wave_height * 0.6;
	VERTEX.y += wave1 + wave2;
	float dx = cos(VERTEX.x * 0.5 + TIME * dynamic_wave_speed) * 0.5 * dynamic_wave_height;
	float dz = cos(VERTEX.z * 0.7 + TIME * dynamic_wave_speed * 0.8) * 0.7 * dynamic_wave_height * 0.6;
	NORMAL = normalize(vec3(-dx, 1.0, -dz));
}

void fragment() {
	float fresnel = pow(1.0 - dot(NORMAL, VIEW), 2.0);
	vec3 albedo = mix(shallow_color, deep_color, fresnel * 0.5);
	vec3 ice_tint = vec3(0.84, 0.92, 0.98);
	vec3 brine_tint = vec3(0.76, 0.68, 0.56);
	vec3 channel_tint = vec3(0.28, 0.32, 0.46);
	albedo = mix(albedo, ice_tint, local_frozen * 0.70);
	albedo = mix(albedo, brine_tint, local_mineral * 0.55);
	albedo = mix(albedo, channel_tint, local_edge * local_liquidness * 0.28);
	float alpha = mix(alpha_min, alpha_max, fresnel);
	alpha = mix(alpha, min(0.99, alpha_max + 0.04), local_frozen * 0.45 + local_mineral * 0.18);
	float roughness = mix(
		roughness_value,
		max(roughness_value, 0.86),
		local_frozen * 0.72 + local_mineral * 0.30
	);
	float specular = mix(specular_value, specular_value * 0.55, local_frozen * 0.50);
	float metallic = mix(metallic_value, metallic_value * 0.35, local_frozen * 0.50);
	ALBEDO = albedo;
	ALPHA = alpha;
	ROUGHNESS = roughness;
	METALLIC = metallic;
	SPECULAR = specular;
}
