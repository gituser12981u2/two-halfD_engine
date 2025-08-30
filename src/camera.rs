pub struct Camera {
    pub pos: [f32; 2], // (x, y) position in world space
    pub yaw: f32,      // radians, camera facing direction in the X-Y plane
    pub eye_z: f32,    // camera height from ground plane
    pub fx: f32,       // horizontal focal factor
    pub fy: f32,       // vertical focal factor
}

impl Camera {
    #[inline]
    pub fn world_to_camera(&self, p: [f32; 2]) -> [f32; 2] {
        // Translate
        let dx = p[0] - self.pos[0];
        let dy = p[1] - self.pos[1];
        // Rotate by -yaw
        let c = self.yaw.cos();
        let s = self.yaw.sin();
        // rotation by -yaw: (x', y') = (x * cos(yaw) + y * sin(yaw), -x * sin(yaw) + y * cos(yaw))
        let cx = dx * c - dy * s;
        let cy = dx * s + dy * c;
        [cx, cy]
    }

    #[inline]
    pub fn project_x(&self, cx: f32, cy: f32, screen_width: f32) -> f32 {
        // center X is half the window width
        let cx0 = 0.5 * screen_width;
        self.fx * (cx / cy) + cx0
    }

    pub fn set_fov_from_horizontal(&mut self, width: f32, height: f32, fov_x_deg: f32) {
        let fov_x = fov_x_deg.to_radians();
        self.fx = 0.5 * width / (0.5 * fov_x).tan();
        let aspect = width / height;
        self.fy = self.fx / aspect;
    }

    #[inline]
    pub fn screen_center_y(&self, screen_h: f32) -> f32 {
        0.5 * screen_h
    }
}
