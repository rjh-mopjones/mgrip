//! Horizontal wrapping utilities for cylindrical world projection.
//! The world wraps horizontally (x-axis) but not vertically (y-axis).

#[inline]
pub fn wrap_x(x: f64, world_width: f64) -> f64 {
    let wrapped = x % world_width;
    if wrapped < 0.0 { wrapped + world_width } else { wrapped }
}

/// Shortest horizontal distance in normalized [0,1] space. Returns [-0.5, 0.5].
#[inline]
pub fn wrapped_dx_normalized(dx: f64) -> f64 {
    if dx > 0.5 { dx - 1.0 } else if dx < -0.5 { dx + 1.0 } else { dx }
}

#[inline]
pub fn wrap_grid_x(nx: i32, width: usize) -> i32 {
    let w = width as i32;
    ((nx % w) + w) % w
}

/// 3D cylindrical noise coordinates for seamless horizontal wrapping.
#[inline]
pub fn cylindrical_noise_coords(x: f64, y: f64, freq: f64, scale: f64, world_width: f64) -> [f64; 3] {
    let angle = std::f64::consts::TAU * x / world_width;
    let r = world_width * freq * scale / std::f64::consts::TAU;
    [angle.cos() * r, angle.sin() * r, y * freq * scale]
}
