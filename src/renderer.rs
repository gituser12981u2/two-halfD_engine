use crate::{camera::Camera, world::World};

const NEAR: f32 = 0.1;

#[inline]
fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    // BGRA8 in little-endian memory
    (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
    // Alpha at 0
}

fn wall_depth_cam_space(cam: &Camera, start: [f32; 2], end: [f32; 2]) -> f32 {
    // Use midpoint depth in camera space for sorting
    let mid = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
    let m = cam.world_to_camera(mid);
    m[1] // cy (forward depth)
}

pub fn render_frame(buf: &mut [u32], width: usize, height: usize, world: &World, camera: &Camera) {
    // Clear background
    let sky = pack_rgb(30, 30, 70);
    let ground = pack_rgb(40, 40, 40);

    let mid = height / 2;
    for y in 0..mid {
        let row = y * width;
        for x in 0..width {
            buf[row + x] = sky;
        }
    }
    for y in mid..height {
        let row = y * width;
        for x in 0..width {
            buf[row + x] = ground;
        }
    }

    // Draw walls
    if world.walls.is_empty() {
        return;
    }

    // sentinels
    let mut ceil_clip: Vec<i32> = vec![height as i32; width]; // “no top yet”
    let mut floor_clip: Vec<i32> = vec![-1; width];

    let mut order: Vec<usize> = (0..world.walls.len()).collect();
    order.sort_by(|&ia, &ib| {
        let wa = &world.walls[ia];
        let wb = &world.walls[ib];
        let da = wall_depth_cam_space(camera, wa.start, wa.end);
        let db = wall_depth_cam_space(camera, wb.start, wb.end);
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal) // farthest first
    });

    let wall_colors = [
        pack_rgb(200, 200, 200),
        pack_rgb(180, 180, 250),
        pack_rgb(250, 180, 180),
        pack_rgb(180, 250, 180),
    ];

    for i in order {
        let wall = &world.walls[i];
        let sector = &world.sectors[wall.front_sector];
        let color = wall_colors[i % wall_colors.len()];
        draw_solid_wall(
            buf,
            width,
            height,
            camera,
            wall,
            sector,
            color,
            &mut ceil_clip,
            &mut floor_clip,
        );
    }

    let ceil_color = pack_rgb(200, 200, 200);
    let floor_color = pack_rgb(30, 30, 70);

    // post fill
    for x in 0..width {
        // draw ceiling only if top is known
        let cc = ceil_clip[x];
        let fc = floor_clip[x];
        if cc >= height as i32 && fc < 0 {
            continue;
        }

        if cc < height as i32 {
            for y in 0..cc.clamp(0, height as i32) {
                buf[y as usize * width + x] = ceil_color;
            }
        }
        // draw floor only if bottom is known
        if fc >= 0 {
            for y in (fc.clamp(-1, height as i32 - 1) + 1)..(height as i32) {
                buf[y as usize * width + x] = floor_color;
            }
        }
    }
}

fn draw_solid_wall(
    buf: &mut [u32],
    width: usize,
    height: usize,
    camera: &Camera,
    wall: &crate::world::Wall,
    sector: &crate::world::Sector,
    color: u32,
    ceil_clip: &mut [i32],
    floor_clip: &mut [i32],
) {
    let screen_width = width as f32;
    let screen_height = height as f32;
    let cy0 = camera.screen_center_y(screen_height);

    // Transform wall endpoints to camera space
    let mut p0 = camera.world_to_camera(wall.start);
    let mut p1 = camera.world_to_camera(wall.end);

    // Trivial reject: both behind near plane
    if p0[1] <= NEAR && p1[1] <= NEAR {
        return;
    }

    // Horizontal frustum reject (fully outside left/right)
    let tan_half_fovx = 0.5 * screen_width / camera.fx;

    let left_plane = |cx: f32, cy: f32| cx < -cy * tan_half_fovx;
    let right_plane = |cx: f32, cy: f32| cx > cy * tan_half_fovx;

    // Both endpoints are on the same outside side, cull
    let p0_left = left_plane(p0[0], p0[1]);
    let p1_left = left_plane(p1[0], p1[1]);
    let p0_right = right_plane(p0[0], p0[1]);
    let p1_right = right_plane(p1[0], p1[1]);

    if (p0_left && p1_left) || (p0_right && p1_right) {
        return; // fully left
    }

    // Clip against near plane (cy > NEAR)
    if !clip_line_near(&mut p0, &mut p1) {
        return; // fully clipped
    }

    let sx0 = camera.project_x(p0[0], p0[1], screen_width);
    let sx1 = camera.project_x(p1[0], p1[1], screen_width);

    // If projected to a single column or entirely off-screen, skip
    if (sx0 - sx1).abs() < 0.5 {
        return;
    }

    // Compute integer screen x range and clamp
    let mut x0 = sx0.floor() as i32;
    let mut x1 = sx1.floor() as i32;
    if x0 > x1 {
        std::mem::swap(&mut x0, &mut x1);
        std::mem::swap(&mut p0, &mut p1); // keep p0/p1 in sync with x0/x1
    }
    let (x0, x1) = (x0.max(0), x1.min((width as i32) - 1));
    if x0 >= x1 {
        return; // off-screen
    }

    // Precompute 1/cy for endpoints
    let inv_cy0 = 1.0 / p0[1];
    let inv_cy1 = 1.0 / p1[1];

    // Left/right screen x after potential swap
    let sx_left = camera.project_x(p0[0], p0[1], screen_width);
    let sx_right = camera.project_x(p1[0], p1[1], screen_width);
    let sx_span = sx_right - sx_left;
    if sx_span.abs() < f32::EPSILON {
        return; // avoid div-by-zero
    }

    // Draw per column
    for xi in x0..=x1 {
        let x = xi as usize;
        let alpha = ((xi as f32) - sx_left) / sx_span; // 0..1 across the wall
        // Interpolate 1/cy at this column
        let inv_cy = inv_lerp(inv_cy0, inv_cy1, alpha);

        let y_to_screen = camera.fy * inv_cy;
        let top = cy0 - y_to_screen * (sector.ceiling_z - camera.eye_z);
        let bottom = cy0 - y_to_screen * (sector.floor_z - camera.eye_z);

        // Clamp to screen
        let mut y0 = top.floor() as i32;
        let mut y1 = bottom.floor() as i32;
        if y0 > y1 {
            std::mem::swap(&mut y0, &mut y1);
        }
        y0 = y0.max(0);
        y1 = y1.min((height as i32) - 1);

        // Vertical draw
        let mut idx = (y0 as usize) * width + x;
        for _y in y0..=y1 {
            buf[idx] = color;
            idx += width;
        }

        if y0 < ceil_clip[x] {
            ceil_clip[x] = y0;
        }
        if y1 > floor_clip[x] {
            floor_clip[x] = y1;
        }
    }
}

#[inline]
fn inv_lerp(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

// Clip line segment in camera space so both endpoints have cy > NEAR
fn clip_line_near(p0: &mut [f32; 2], p1: &mut [f32; 2]) -> bool {
    let mut in0 = p0[1] > NEAR;
    let mut in1 = p1[1] > NEAR;

    if !in0 && !in1 {
        return false; // both behind
    }

    // Intersect a point 'a' against the near plane along segment a->b
    let clip_endpoint = |a: &mut [f32; 2], b: &[f32; 2]| {
        let t = (NEAR - a[1]) / (b[1] - a[1]);
        a[0] = a[0] + t * (b[0] - a[0]);
        a[1] = NEAR;
    };

    if !in0 && in1 {
        clip_endpoint(p0, p1);
        in0 = true;
    } else if in0 && !in1 {
        clip_endpoint(p1, p0);
        in1 = true;
    }

    in0 && in1
}
