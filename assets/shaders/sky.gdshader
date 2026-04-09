shader_type sky;
render_mode disable_fog;

uniform vec3 scatter_coeffs = vec3(0.5, 1.2, 2.8);
uniform float rayleigh_strength = 1.0;
uniform float mie_strength = 1.0;
uniform float atmosphere_density = 1.0;
uniform vec3 mie_colour : source_color = vec3(0.98, 0.62, 0.28);
uniform float mie_g = 0.76;

uniform float night_blend = 0.0;
uniform float aurora_intensity = 0.0;
uniform vec3 aurora_colour_a : source_color = vec3(0.10, 0.90, 0.30);
uniform vec3 aurora_colour_b : source_color = vec3(0.72, 0.14, 0.86);
uniform vec3 ground_glow_colour : source_color = vec3(0.08, 0.04, 0.06);
uniform float ground_glow_intensity = 0.4;

const float RAYLEIGH_SCALE   = 0.012;
const float RAYLEIGH_SCATTER = 0.025;
const float MIE_SCATTER      = 0.010;

float rayleigh_phase(float cos_theta) {
	return 0.75 * (1.0 + cos_theta * cos_theta);
}

float mie_phase(float cos_theta, float g) {
	float g2 = g * g;
	return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 0.0001), 1.5);
}

float star_field(vec3 dir) {
	if (dir.y < 0.0) return 0.0;
	vec3 d = floor(dir * 280.0);
	float h = fract(sin(dot(d, vec3(127.1, 311.7, 74.3))) * 43758.5453);
	float brightness = step(0.997, h);
	// vary star brightness so they don't all look identical
	float vary = fract(sin(dot(d, vec3(43.1, 217.3, 31.7))) * 9381.2);
	return brightness * (0.5 + 0.5 * vary) * smoothstep(0.15, 0.55, night_blend);
}

float aurora(vec3 dir) {
	if (dir.y < 0.04) return 0.0;
	// bias toward nightside (-Z on Margin)
	float north_factor = smoothstep(0.25, -0.45, dir.z);
	float lat = dir.y;
	float lon = atan(dir.x, dir.z);
	// two layered waves; abs() avoids hard cutoff edges
	float wave = abs(sin(lon * 3.1 + TIME * 0.27) * sin(lon * 7.3 - TIME * 0.11));
	wave += 0.35 * abs(sin(lon * 5.0 - TIME * 0.18) * sin(lon * 2.1 + TIME * 0.09));
	float curtain = smoothstep(0.04, 0.40, lat) * smoothstep(0.88, 0.42, lat);
	return wave * curtain * north_factor;
}

void sky() {
	vec3 sun_dir  = -LIGHT0_DIRECTION;
	float sun_elev = dot(sun_dir, vec3(0.0, 1.0, 0.0));
	float cos_theta = dot(EYEDIR, sun_dir);

	float view_optical = atmosphere_density / max(EYEDIR.y + 0.12, 0.04);
	float sun_optical  = atmosphere_density / max(sun_elev  + 0.12, 0.04);

	vec3 beta_r = scatter_coeffs * rayleigh_strength;
	vec3 sun_transmit = exp(-sun_optical * beta_r * RAYLEIGH_SCALE);

	float r_phase = rayleigh_phase(cos_theta);
	float m_phase = mie_phase(cos_theta, mie_g);

	vec3 sky_col = r_phase * beta_r * sun_transmit * view_optical * RAYLEIGH_SCATTER;
	sky_col += m_phase * mie_strength * mie_colour * sun_transmit * view_optical * MIE_SCATTER;

	// Sun disc — boosted brightness so it punches through tonemapping
	if (LIGHT0_ENABLED && sun_elev > -0.04) {
		float angle_to_sun = acos(clamp(cos_theta, -1.0, 1.0));
		float disc_size = max(LIGHT0_SIZE, 0.045);
		float disc = 1.0 - smoothstep(disc_size * 0.3, disc_size, angle_to_sun);
		// halo ring around disc for visibility
		float halo = 1.0 - smoothstep(disc_size, disc_size * 3.5, angle_to_sun);
		sky_col += LIGHT0_COLOR * (disc * 3.5 + halo * 0.4) * sun_transmit;
	}

	// Night blend — full dome, dayside scattering preserved at terminus
	// Softer transition: full night only below light_level 0.25 (night_blend > 0.8 range)
	vec3 night_sky = vec3(0.004, 0.002, 0.010);
	sky_col = mix(sky_col, night_sky, night_blend);

	// Stars — above horizon, fade in with night_blend
	float stars = star_field(EYEDIR);
	sky_col += vec3(stars * 0.9);

	// Aurora curtains — nightside only, scaled by aurora_intensity
	float aur = aurora(EYEDIR);
	float slow_wave = sin(EYEDIR.x * 4.2 + TIME * 0.06) * 0.5 + 0.5;
	vec3 aur_col = mix(aurora_colour_a, aurora_colour_b, slow_wave);
	sky_col += aur_col * aur * aurora_intensity * 0.6;

	// Ground glow — below horizon, applied last so it's never overridden
	float below = 1.0 - smoothstep(-0.28, 0.0, EYEDIR.y);
	sky_col = mix(sky_col, ground_glow_colour * ground_glow_intensity, below * 0.88);

	COLOR = sky_col;
}
