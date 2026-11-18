// The Velouna mark: two velvet drapes that fall together into a V.
// Shared by the app (window icon, in-app logo) and build.rs (Windows .ico),
// so it only uses std and plain `//` comments (it is `include!`d by build.rs).

/// A point in the unit square, y pointing down.
pub type LogoPoint = (f32, f32);

/// Brand colours (RGBA) of the app icon.
pub const LOGO_TILE: [u8; 4] = [0x1A, 0x12, 0x20, 0xFF];
pub const LOGO_DRAPE: [u8; 4] = [0xD8, 0x3F, 0x5C, 0xFF];

/// Brightness of the fold lines relative to the drape colour.
const FOLD_SHADE: f32 = 0.74;

/// Square region of the icon tile that holds the mark (x0, y0, size).
pub const LOGO_CONTENT: (f32, f32, f32) = (0.125, 0.125, 0.75);

fn logo_quad(a: LogoPoint, c: LogoPoint, b: LogoPoint, n: usize) -> Vec<LogoPoint> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1)
        })
        .collect()
}

/// Polygons of the mark in the unit square (any shape, filled even-odd),
/// painted in order, with their shade (1.0 = drape colour).
pub fn logo_shapes() -> Vec<(Vec<LogoPoint>, f32)> {
    let p = |x: f32, y: f32| (x / 96.0, y / 96.0);
    // Left drape: wide at the curtain rod, gathered into the point of the V.
    let mut left = logo_quad(p(12.0, 14.0), p(27.0, 52.0), p(46.0, 82.0), 24);
    left.extend(logo_quad(p(50.0, 82.0), p(39.0, 47.0), p(31.0, 14.0), 24));
    // A fold running down the drape gives it the velvet depth.
    let fold_line = logo_quad(p(21.0, 15.0), p(32.0, 48.0), p(47.0, 80.0), 24);
    let half = 1.1 / 96.0;
    let mut fold: Vec<LogoPoint> = fold_line.iter().map(|&(x, y)| (x - half, y)).collect();
    fold.extend(fold_line.iter().rev().map(|&(x, y)| (x + half, y)));
    let mirror = |v: &[LogoPoint]| v.iter().map(|&(x, y)| (1.0 - x, y)).collect::<Vec<_>>();
    vec![(mirror(&left), 1.0), (left.clone(), 1.0), (mirror(&fold), FOLD_SHADE), (fold, FOLD_SHADE)]
}

fn logo_inside(poly: &[LogoPoint], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn logo_in_tile(x: f32, y: f32, r: f32) -> bool {
    let cx = x.clamp(r, 1.0 - r);
    let cy = y.clamp(r, 1.0 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// Shade of the mark at a point (None = background).
fn logo_shade(shapes: &[(Vec<LogoPoint>, f32)], x: f32, y: f32) -> Option<f32> {
    let mut s = None;
    for (poly, shade) in shapes {
        if logo_inside(poly, x, y) {
            s = Some(*shade);
        }
    }
    s
}

/// Supersamples `n`×`n` pixels over the unit-square window (x0, y0, size);
/// `sample` returns an RGB colour or None for transparent.
fn logo_render(n: usize, win: (f32, f32, f32), sample: impl Fn(f32, f32) -> Option<[f32; 3]>) -> Vec<u8> {
    let ss = 4usize;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0f32; 3];
            let mut hits = 0f32;
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = win.0 + (x as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32 * win.2;
                    let py = win.1 + (y as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32 * win.2;
                    if let Some(c) = sample(px, py) {
                        for k in 0..3 {
                            acc[k] += c[k];
                        }
                        hits += 1.0;
                    }
                }
            }
            if hits > 0.0 {
                let i = (y * n + x) * 4;
                for k in 0..3 {
                    out[i + k] = (acc[k] / hits).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (hits / (ss * ss) as f32 * 255.0).round() as u8;
            }
        }
    }
    out
}

/// The app icon (mark on its tile) as straight RGBA, `n`×`n`.
pub fn logo_rasterize(n: usize) -> Vec<u8> {
    let shapes = logo_shapes();
    let tile = [LOGO_TILE[0] as f32, LOGO_TILE[1] as f32, LOGO_TILE[2] as f32];
    logo_render(n, (0.0, 0.0, 1.0), |x, y| {
        if !logo_in_tile(x, y, 0.22) {
            return None;
        }
        Some(match logo_shade(&shapes, x, y) {
            Some(s) => [LOGO_DRAPE[0] as f32 * s, LOGO_DRAPE[1] as f32 * s, LOGO_DRAPE[2] as f32 * s],
            None => tile,
        })
    })
}

/// The bare mark as a white (shaded) mask with alpha, `n`×`n`, cropped to
/// `LOGO_CONTENT`; tint it with any colour.
pub fn logo_mask(n: usize) -> Vec<u8> {
    let shapes = logo_shapes();
    logo_render(n, LOGO_CONTENT, |x, y| logo_shade(&shapes, x, y).map(|s| [255.0 * s; 3]))
}
