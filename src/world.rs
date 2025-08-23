pub struct Sector {
    pub floor_z: f32,
    pub ceiling_z: f32,
}

pub struct Wall {
    pub start: [f32; 2], // (x, y) start point in world space
    pub end: [f32; 2],   // (x, y) end point in world space
    pub front_sector: usize,
    pub back_sector: Option<usize>, // None if one-sided wall
}

pub struct World {
    pub sectors: Vec<Sector>,
    pub walls: Vec<Wall>,
}
