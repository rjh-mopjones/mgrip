shader_type sky;
render_mode disable_fog;

// Scattering coefficients — no source_color hint, these are physical coefficients not colours.
// B > G > R so blue light is stripped from the sun path faster than red at low sun angles,
// producing horizon reddening. Margin's atmosphere is alien — tune these for the world.
uniform vec3 scatter_coeffs = vec3(0.5, 1.2, 2.8);
uniform float rayleigh_strength = 1.0;
uniform float mie_strength = 1.0;
uniform float atmosphere_density = 1.0;
uniform vec3 mie_colour : source_color = vec3(0.98, 0.62, 0.28);
uniform float mie_g = 0.76; // Henyey-Greenstein: ~0.85 tight corona, ~0.55 diffuse haze

// Nightside — Phase 2 uniforms, wired now so Phase 2 only needs to set values
uniform float night_blend = 0.0;
uniform float aurora_intensity = 0.0;
uniform vec3 aurora_colour_a : source_color = vec3(0.10, 0.90, 0.30); // oxygen green
uniform vec3 aurora_colour_b : source_color = vec3(0.72, 0.14, 0.86); // high-altitude purple
uniform vec3 ground_glow_colour : source_color = vec3(0.08, 0.04, 0.06);
uniform float ground_glow_intensity = 0.4;

// Scale constants — named so they can be tuned without hunting for magic numbers
const float RAYLEIGH_SCALE   = 0.01;  // optical depth scale for sun transmittance
const float RAYLEIGH_SCATTER = 0.02;  // sky dome scatter brightness
const float MIE_SCATTER      = 0.008; // sun halo brightness

float rayleigh_phase(float cos_theta) {
	return 0.75 * (1.0 + cos_theta * cos_theta);
}

float mie_phase(float cos_theta, float g) {
	float g2 = g * g;
	// Clamp denominator to avoid divide-by-zero at exact g=1
	return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 0.0001), 1.5);
}

void sky() {
	// In Godot sky shaders, LIGHT0_DIRECTION is the direction light travels (source -> scene).
	// Negate it to get the direction from the scene toward the sun.
	vec3 sun_dir  = -LIGHT0_DIRECTION;
	float sun_elev = dot(sun_dir, vec3(0.0, 1.0, 0.0)); // 0.0 = horizon, 1.0 = overhead
	float cos_theta = dot(EYEDIR, sun_dir);

	// Two separate optical depth paths — this is the mechanism that produces terminus reddening.
	// When sun_elev is near 0 (sun on horizon), sun_optical is large, exp(-sun_optical * beta_r)
	// strips blue wavelengths far more than red, reddening the sunlight before it scatters.
	float view_optical = atmosphere_density / max(EYEDIR.y + 0.12, 0.04);
	float sun_optical  = atmosphere_density / max(sun_elev  + 0.12, 0.04);

	vec3 beta_r = scatter_coeffs * rayleigh_strength;

	// Wavelength-dependent transmittance along the sun's atmospheric path
	vec3 sun_transmit = exp(-sun_optical * beta_r * RAYLEIGH_SCALE);

	float r_phase = rayleigh_phase(cos_theta);
	float m_phase = mie_phase(cos_theta, mie_g);

	// Sky dome colour from Rayleigh-scattered sunlight
	vec3 sky_col = r_phase * beta_r * sun_transmit * view_optical * RAYLEIGH_SCATTER;

	// Mie haze — glow and corona around the sun direction
	sky_col += m_phase * mie_strength * mie_colour * sun_transmit * view_optical * MIE_SCATTER;

	// Sun disc — only renders when sun is above the horizon
	if (LIGHT0_ENABLED && sun_elev > -0.04) {
		float angle_to_sun = acos(clamp(cos_theta, -1.0, 1.0));
		float disc_size = max(LIGHT0_SIZE, 0.035); // fallback if angular_distance not set on light
		float disc = 1.0 - smoothstep(disc_size * 0.35, disc_size, angle_to_sun);
		sky_col += LIGHT0_COLOR * disc * 1.4 * sun_transmit;
	}

	// Night blend — applied to the full dome so the nightside is dark from all angles.
	// Ground glow is applied after so it shows through regardless.
	vec3 night_sky = vec3(0.004, 0.002, 0.010);
	sky_col = mix(sky_col, night_sky, night_blend);

	// Ground glow — applied last so it is never overridden by night_blend
	float below = 1.0 - smoothstep(-0.25, 0.0, EYEDIR.y);
	sky_col = mix(sky_col, ground_glow_colour * ground_glow_intensity, below * 0.9);

	COLOR = sky_col;
}
