use rayon::{
    iter::{IndexedParallelIterator, ParallelIterator},
    slice::ParallelSliceMut,
};

/// Precomputed mapping from dest pixels to src neighbors + weights
pub struct ScaleLut {
    x0: Vec<usize>,
    x1: Vec<usize>,
    wx: Vec<u16>,
    y0: Vec<usize>,
    y1: Vec<usize>,
    wy: Vec<u16>,
}

impl ScaleLut {
    pub fn empty() -> Self {
        Self {
            x0: Vec::new(),
            x1: Vec::new(),
            wx: Vec::new(),
            y0: Vec::new(),
            y1: Vec::new(),
            wy: Vec::new(),
        }
    }
}

pub fn build_scale_lut(dst_w: usize, dst_h: usize, src_w: usize, src_h: usize) -> ScaleLut {
    let mut x0 = vec![0; dst_w];
    let mut x1 = vec![0; dst_w];
    let mut wx = vec![0; dst_w];
    let mut y0 = vec![0; dst_h];
    let mut y1 = vec![0; dst_h];
    let mut wy = vec![0; dst_h];

    let sx = src_w as f32 / dst_w as f32;
    let sy = src_h as f32 / dst_h as f32;

    for x in 0..dst_w {
        let fx = x as f32 * sx;
        let x0_val = fx.floor() as isize;
        let x1_val = (x0_val + 1).clamp(0, src_w as isize - 1);
        x0[x] = x0_val as usize;
        x1[x] = x1_val as usize;
        wx[x] = ((fx - x0_val as f32) * 256.0).round() as u16; // fixed-point 8.8
    }

    for y in 0..dst_h {
        let fy = y as f32 * sy;
        let y0_val = fy.floor() as isize;
        let y1_val = (y0_val + 1).clamp(0, src_h as isize - 1);
        y0[y] = y0_val as usize;
        y1[y] = y1_val as usize;
        wy[y] = ((fy - y0_val as f32) * 256.0).round() as u16; // fixed-point 8.8
    }

    ScaleLut {
        x0,
        x1,
        wx,
        y0,
        y1,
        wy,
    }
}

#[inline]
fn lerp_color_u32(a: u32, b: u32, w256: u32) -> u32 {
    // w256 in [0, 256]; inv = 256 - w256
    let inv = 256 - w256;
    // Interpolate R and B together (00RR00BB), with mask 0x00FF00FF,
    let rb = ((a & 0x00FF00FF) * inv + (b & 0x00FF00FF) * w256) >> 8 & 0x00FF00FF;
    // Interpolate G separately (0000GG00), with mask 0x0000FF00
    let g = ((a & 0x0000FF00) * inv + (b & 0x0000FF00) * w256) >> 8 & 0x0000FF00;
    rb | g // alpha stays 0
}

/// Parallel bilinear stretch
/// Rows are processed in parallel for cache friendly writes
pub fn blit_bilinear_stretch(dst: &mut [u32], dw: usize, src: &[u32], sw: usize, lut: &ScaleLut) {
    dst.par_chunks_mut(dw).enumerate().for_each(|(y, dst_row)| {
        let y0 = lut.y0[y];
        let y1 = lut.y1[y];
        let wy = lut.wy[y] as u32;
        let row0 = y0 * sw;
        let row1 = y1 * sw;

        for x in 0..dw {
            let x0 = lut.x0[x];
            let x1 = lut.x1[x];
            let wx = lut.wx[x] as u32;

            // read 4 neighbors
            let c00 = src[row0 + x0];
            let c10 = src[row0 + x1];
            let c01 = src[row1 + x0];
            let c11 = src[row1 + x1];

            // horizontal lerp
            let top = lerp_color_u32(c00, c10, wx);
            let bot = lerp_color_u32(c01, c11, wx);
            // vertical lerp
            dst_row[x] = lerp_color_u32(top, bot, wy);
        }
    });
}

/// Cross-shaped 3x3 sharpen
pub fn sharpen3x3_cross_inplace(dst: &mut [u32], w: usize, h: usize) {
    if w < 3 || h < 3 {
        return;
    }
    let src = dst.to_vec();

    // top/bottom rows unchanged
    // use parallel rows for y = 1..h-2
    dst.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y == 0 || y == h - 1 {
            row.copy_from_slice(&src[y * w..(y + 1) * w]);
            return;
        }

        // left/right borders unchanged
        row[0] = src[y * w];
        row[w - 1] = src[y * w + (w - 1)];

        for x in 1..(w - 1) {
            let c = src[y * w + x];
            let n = src[(y - 1) * w + x];
            let s = src[(y + 1) * w + x];
            let e = src[y * w + (x + 1)];
            let wv = src[y * w + (x - 1)];

            // per channel integer math
            let (cb, cg, cr) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF);
            let (nb, ng, nr) = (n & 0xFF, (n >> 8) & 0xFF, (n >> 16) & 0xFF);
            let (sb, sg, sr) = (s & 0xFF, (s >> 8) & 0xFF, (s >> 16) & 0xFF);
            let (eb, eg, er) = (e & 0xFF, (e >> 8) & 0xFF, (e >> 16) & 0xFF);
            let (wb, wg, wr) = (wv & 0xFF, (wv >> 8) & 0xFF, (wv >> 16) & 0xFF);

            #[inline]
            fn sat8(v: i32) -> u32 {
                v.clamp(0, 255) as u32
            }

            let rb = 5 * (cr as i32) - (nr as i32 + sr as i32 + er as i32 + wr as i32);
            let gb = 5 * (cg as i32) - (ng as i32 + sg as i32 + eg as i32 + wg as i32);
            let bb = 5 * (cb as i32) - (nb as i32 + sb as i32 + eb as i32 + wb as i32);

            let r = sat8(rb);
            let g = sat8(gb);
            let b = sat8(bb);

            row[x] = (r << 16) | (g << 8) | b;
        }
    });
}
