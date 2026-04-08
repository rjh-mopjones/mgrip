shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_disabled, diffuse_burley, specular_schlick_ggx;

uniform vec3  shallow_color : source_color = vec3(0.16, 0.20, 0.42);
uniform vec3  deep_color    : source_color = vec3(0.04, 0.05, 0.16);
uniform float wave_speed    = 0.8;
uniform float wave_height   = 0.15;

void vertex() {
	float wave1 = sin(VERTEX.x * 0.5 + TIME * wave_speed) * wave_height;
	float wave2 = sin(VERTEX.z * 0.7 + TIME * wave_speed * 0.8) * wave_height * 0.6;
	VERTEX.y += wave1 + wave2;
	float dx = cos(VERTEX.x * 0.5 + TIME * wave_speed) * 0.5 * wave_height;
	float dz = cos(VERTEX.z * 0.7 + TIME * wave_speed * 0.8) * 0.7 * wave_height * 0.6;
	NORMAL = normalize(vec3(-dx, 1.0, -dz));
}

void fragment() {
	float fresnel = pow(1.0 - dot(NORMAL, VIEW), 2.0);
	ALBEDO    = mix(shallow_color, deep_color, fresnel * 0.5);
	ALPHA     = mix(0.45, 0.88, fresnel);
	ROUGHNESS = 0.15;
	METALLIC  = 0.1;
	SPECULAR  = 0.5;
}
