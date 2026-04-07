shader_type sky;
render_mode disable_fog;

uniform vec3 zenith_color : source_color = vec3(0.015, 0.005, 0.028);
uniform vec3 upper_color : source_color = vec3(0.120, 0.025, 0.040);
uniform vec3 horizon_color : source_color = vec3(0.620, 0.180, 0.080);
uniform vec3 haze_color : source_color = vec3(0.980, 0.420, 0.140);
uniform float horizon_height = -0.08;
uniform float horizon_softness = 0.30;
uniform float sun_disc_size = 0.045;
uniform float sun_glow_size = 0.22;

void sky() {
	float up = clamp(EYEDIR.y * 0.5 + 0.5, 0.0, 1.0);
	float horizon = smoothstep(horizon_height - horizon_softness, horizon_height + horizon_softness, EYEDIR.y);
	vec3 sky_col = mix(horizon_color, upper_color, pow(horizon, 0.75));
	sky_col = mix(sky_col, zenith_color, pow(up, 1.8));
	float below_horizon = 1.0 - smoothstep(-0.30, 0.06, EYEDIR.y);
	vec3 ground_col = mix(vec3(0.03, 0.01, 0.02), horizon_color * 0.32, smoothstep(-0.30, 0.0, EYEDIR.y));
	sky_col = mix(sky_col, ground_col, below_horizon * 0.85);

	if (LIGHT0_ENABLED) {
		vec3 sun_dir = normalize(-LIGHT0_DIRECTION);
		float sun_dot = max(dot(EYEDIR, sun_dir), 0.0);
		float disc = smoothstep(cos(sun_disc_size), cos(sun_disc_size * 0.35), sun_dot);
		float glow = pow(sun_dot, 1.0 / max(sun_glow_size, 0.001));
		vec3 glow_col = haze_color * (0.55 + 0.45 * LIGHT0_COLOR);
		sky_col += glow_col * glow * 0.35;
		sky_col += vec3(1.0, 0.52, 0.18) * disc * 1.4;
	}

	float low_haze = 1.0 - smoothstep(0.02, 0.50, up);
	sky_col = mix(sky_col, haze_color, low_haze * 0.18);
	COLOR = sky_col;
}
