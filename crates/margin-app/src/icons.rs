//! Icons: GitHub mark and note, from Primer Octicons (MIT, © GitHub Inc.).
//!
//! Instead of pulling in an SVG library (resvg ≈ 1–2 MB), the SVG path data
//! is rasterised here by a ~150-line scanline filler at the exact pixel size
//! needed (crisp on Retina), cached as white alpha-mask textures and tinted
//! at draw time.

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, load::SizedTexture, pos2, vec2};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Icon {
    Github,
    Note,
}

// octicons 19.42.0: mark-github-16/24, note-16/24 (path data verbatim).
const GITHUB_16: &str = "M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 .468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 1-.5c.266-.265.47-.5.657-.656";
const GITHUB_24: &str = "M10.226 17.284c-2.965-.36-5.054-2.493-5.054-5.256 0-1.123.404-2.336 1.078-3.144-.292-.741-.247-2.314.09-2.965.898-.112 2.111.36 2.83 1.01.853-.269 1.752-.404 2.853-.404 1.1 0 1.999.135 2.807.382.696-.629 1.932-1.1 2.83-.988.315.606.36 2.179.067 2.942.72.854 1.101 2 1.101 3.167 0 2.763-2.089 4.852-5.098 5.234.763.494 1.28 1.572 1.28 2.807v2.336c0 .674.561 1.056 1.235.786 4.066-1.55 7.255-5.615 7.255-10.646C23.5 6.188 18.334 1 11.978 1 5.62 1 .5 6.188.5 12.545c0 4.986 3.167 9.12 7.435 10.669.606.225 1.19-.18 1.19-.786V20.63a2.9 2.9 0 0 1-1.078.224c-1.483 0-2.359-.808-2.987-2.313-.247-.607-.517-.966-1.034-1.033-.27-.023-.359-.135-.359-.27 0-.27.45-.471.898-.471.652 0 1.213.404 1.797 1.235.45.651.921.943 1.483.943.561 0 .92-.202 1.437-.719.382-.381.674-.718.944-.943";
const NOTE_16: &str = "M0 3.75C0 2.784.784 2 1.75 2h12.5c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 14H1.75A1.75 1.75 0 0 1 0 12.25Zm1.75-.25a.25.25 0 0 0-.25.25v8.5c0 .138.112.25.25.25h12.5a.25.25 0 0 0 .25-.25v-8.5a.25.25 0 0 0-.25-.25ZM3.5 6.25a.75.75 0 0 1 .75-.75h7a.75.75 0 0 1 0 1.5h-7a.75.75 0 0 1-.75-.75Zm.75 2.25h4a.75.75 0 0 1 0 1.5h-4a.75.75 0 0 1 0-1.5Z";
const NOTE_24: &str = "M0 4.75C0 3.784.784 3 1.75 3h20.5c.966 0 1.75.784 1.75 1.75v14.5A1.75 1.75 0 0 1 22.25 21H1.75A1.75 1.75 0 0 1 0 19.25Zm1.75-.25a.25.25 0 0 0-.25.25v14.5c0 .138.112.25.25.25h20.5a.25.25 0 0 0 .25-.25V4.75a.25.25 0 0 0-.25-.25Z M5 8.75A.75.75 0 0 1 5.75 8h11.5a.75.75 0 0 1 0 1.5H5.75A.75.75 0 0 1 5 8.75Zm0 4a.75.75 0 0 1 .75-.75h5.5a.75.75 0 0 1 0 1.5h-5.5a.75.75 0 0 1-.75-.75Z";

fn source(icon: Icon, px: usize) -> (&'static str, f32) {
    // Octicons ship hand-tuned 16 and 24 grids; use the closer one.
    match (icon, px <= 20) {
        (Icon::Github, true) => (GITHUB_16, 16.0),
        (Icon::Github, false) => (GITHUB_24, 24.0),
        (Icon::Note, true) => (NOTE_16, 16.0),
        (Icon::Note, false) => (NOTE_24, 24.0),
    }
}

#[derive(Clone, Default)]
struct Cache(HashMap<(Icon, usize), TextureHandle>);

/// Texture for `icon` drawn at `size` points (square).
pub fn texture(ctx: &egui::Context, icon: Icon, size: f32) -> SizedTexture {
    let px = (size * ctx.pixels_per_point()).round().max(4.0) as usize;
    let id = egui::Id::unique("margin_icon_cache");
    let cached = ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<Cache>(id).0.get(&(icon, px)).cloned());
    let tex = match cached {
        Some(t) => t,
        None => {
            let (d, view) = source(icon, px);
            let alpha = rasterize(d, view, px);
            let img = ColorImage::new([px, px], alpha.into_iter().map(Color32::from_white_alpha).collect());
            let t = ctx.load_texture(format!("icon-{icon:?}-{px}"), img, TextureOptions::LINEAR);
            ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<Cache>(id).0.insert((icon, px), t.clone()));
            t
        }
    };
    SizedTexture::new(tex.id(), vec2(size, size))
}

/// Paint `icon` centred in `rect` (square of the smaller side), tinted.
pub fn paint(p: &egui::Painter, icon: Icon, rect: Rect, tint: Color32) {
    let s = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(s, s));
    let tex = texture(p.ctx(), icon, s);
    p.image(tex.id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
}

/// An `egui::Image` widget/atom for labels, tabs and buttons.
pub fn image(ctx: &egui::Context, icon: Icon, size: f32, tint: Color32) -> egui::Image<'static> {
    egui::Image::from_texture(texture(ctx, icon, size)).tint(tint)
}

// ------------------------------------------------------------- rasteriser

/// Fill an SVG path (nonzero rule) into a `px`×`px` alpha mask, 4×4 supersampled.
pub fn rasterize(d: &str, view: f32, px: usize) -> Vec<u8> {
    const SS: usize = 4;
    let scale = px as f32 / view;
    let polys: Vec<Vec<(f32, f32)>> = flatten(d)
        .into_iter()
        .map(|p| p.into_iter().map(|(x, y)| (x * scale, y * scale)).collect())
        .collect();
    let mut cov = vec![0u16; px * px];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for row in 0..px {
        for sub in 0..SS {
            let y = row as f32 + (sub as f32 + 0.5) / SS as f32;
            xs.clear();
            for poly in &polys {
                for i in 0..poly.len() {
                    let (x0, y0) = poly[i];
                    let (x1, y1) = poly[(i + 1) % poly.len()];
                    if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                        let t = (y - y0) / (y1 - y0);
                        xs.push((x0 + t * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
                    }
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for w in xs.windows(2) {
                wind += w[0].1;
                if wind == 0 {
                    continue;
                }
                let (a, b) = (w[0].0.max(0.0), w[1].0.min(px as f32));
                // count horizontal sample points (c + (j+.5)/SS) inside [a, b)
                let first = ((a * SS as f32) - 0.5).ceil().max(0.0) as usize;
                let last = ((b * SS as f32) - 0.5).ceil().max(0.0) as usize;
                for s in first..last.min(px * SS) {
                    cov[row * px + s / SS] += 1;
                }
            }
        }
    }
    cov.into_iter().map(|c| ((c as u32 * 255) / (SS * SS) as u32) as u8).collect()
}

/// Parse SVG path data into closed polylines (curves and arcs flattened).
fn flatten(d: &str) -> Vec<Vec<(f32, f32)>> {
    let mut t = Tok { s: d.as_bytes(), i: 0 };
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut last_ctrl: Option<(f32, f32)> = None; // for S/T reflection
    let mut cmd = b'M';
    let close = |cur: &mut Vec<(f32, f32)>, polys: &mut Vec<Vec<(f32, f32)>>| {
        if cur.len() > 2 {
            polys.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    loop {
        t.skip_ws();
        if t.i >= t.s.len() {
            break;
        }
        let c = t.s[t.i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            t.i += 1;
        } else if cmd == b'M' || cmd == b'm' {
            // implicit lineto after moveto
            cmd = if cmd == b'M' { b'L' } else { b'l' };
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                close(&mut cur, &mut polys);
                let (nx, ny) = (t.num() + ox, t.num() + oy);
                x = nx;
                y = ny;
                sx = x;
                sy = y;
                cur.push((x, y));
                last_ctrl = None;
            }
            b'L' => {
                x = t.num() + ox;
                y = t.num() + oy;
                cur.push((x, y));
                last_ctrl = None;
            }
            b'H' => {
                x = t.num() + if rel { x } else { 0.0 };
                cur.push((x, y));
                last_ctrl = None;
            }
            b'V' => {
                y = t.num() + if rel { y } else { 0.0 };
                cur.push((x, y));
                last_ctrl = None;
            }
            b'C' | b'S' => {
                let (c1x, c1y) = if cmd.to_ascii_uppercase() == b'C' {
                    (t.num() + ox, t.num() + oy)
                } else {
                    last_ctrl.map(|(cx, cy)| (2.0 * x - cx, 2.0 * y - cy)).unwrap_or((x, y))
                };
                let (c2x, c2y) = (t.num() + ox, t.num() + oy);
                let (ex, ey) = (t.num() + ox, t.num() + oy);
                for k in 1..=12 {
                    let u = k as f32 / 12.0;
                    let v = 1.0 - u;
                    cur.push((
                        v * v * v * x + 3.0 * v * v * u * c1x + 3.0 * v * u * u * c2x + u * u * u * ex,
                        v * v * v * y + 3.0 * v * v * u * c1y + 3.0 * v * u * u * c2y + u * u * u * ey,
                    ));
                }
                last_ctrl = Some((c2x, c2y));
                x = ex;
                y = ey;
            }
            b'Q' | b'T' => {
                let (cx, cy) = if cmd.to_ascii_uppercase() == b'Q' {
                    (t.num() + ox, t.num() + oy)
                } else {
                    last_ctrl.map(|(cx, cy)| (2.0 * x - cx, 2.0 * y - cy)).unwrap_or((x, y))
                };
                let (ex, ey) = (t.num() + ox, t.num() + oy);
                for k in 1..=10 {
                    let u = k as f32 / 10.0;
                    let v = 1.0 - u;
                    cur.push((v * v * x + 2.0 * v * u * cx + u * u * ex, v * v * y + 2.0 * v * u * cy + u * u * ey));
                }
                last_ctrl = Some((cx, cy));
                x = ex;
                y = ey;
            }
            b'A' => {
                let (rx, ry, rot) = (t.num(), t.num(), t.num());
                let (large, sweep) = (t.flag(), t.flag());
                let (ex, ey) = (t.num() + ox, t.num() + oy);
                arc(&mut cur, (x, y), (ex, ey), rx, ry, rot, large, sweep);
                x = ex;
                y = ey;
                last_ctrl = None;
            }
            b'Z' => {
                close(&mut cur, &mut polys);
                x = sx;
                y = sy;
                cur.push((x, y));
                last_ctrl = None;
                // A command letter must follow Z; avoid looping on stray numbers.
                t.skip_ws();
                if t.i < t.s.len() && !t.s[t.i].is_ascii_alphabetic() {
                    t.num();
                }
            }
            _ => {
                t.i += 1; // unknown: skip
            }
        }
    }
    close(&mut cur, &mut polys);
    polys
}

/// SVG endpoint arc → points (spec F.6.5).
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<(f32, f32)>, p0: (f32, f32), p1: (f32, f32), rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || p0 == p1 {
        out.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (cs, sn) = (phi.cos(), phi.sin());
    let dx = (p0.0 - p1.0) / 2.0;
    let dy = (p0.1 - p1.1) / 2.0;
    let x1 = cs * dx + sn * dy;
    let y1 = -sn * dx + cs * dy;
    let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = (num / den).sqrt();
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1 / ry;
    let cyp = -co * ry * x1 / rx;
    let cx = cs * cxp - sn * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sn * cxp + cs * cyp + (p0.1 + p1.1) / 2.0;
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let a = (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
        a
    };
    let t1 = ang(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut dt = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f32::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f32::consts::TAU;
    }
    let n = ((dt.abs() / (std::f32::consts::PI / 16.0)).ceil() as usize).max(2);
    for k in 1..=n {
        let a = t1 + dt * k as f32 / n as f32;
        let (ca, sa) = (a.cos(), a.sin());
        out.push((cx + cs * rx * ca - sn * ry * sa, cy + sn * rx * ca + cs * ry * sa));
    }
}

struct Tok<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tok<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }
    fn flag(&mut self) -> bool {
        self.skip_ws();
        let f = self.s.get(self.i) == Some(&b'1');
        self.i += 1;
        f
    }
    fn num(&mut self) -> f32 {
        self.skip_ws();
        let start = self.i;
        let s = self.s;
        if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < s.len() {
            match s[self.i] {
                b'0'..=b'9' => {}
                b'.' if !dot => dot = true,
                b'e' | b'E' => {
                    self.i += 1;
                    if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
                        self.i += 1;
                    }
                    continue;
                }
                _ => break,
            }
            self.i += 1;
        }
        std::str::from_utf8(&s[start..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_icon_shape() {
        // 16px note: frame + hollow inside + two text lines.
        let a = rasterize(NOTE_16, 16.0, 16);
        let at = |x: usize, y: usize| a[y * 16 + x];
        assert!(at(0, 8) > 200, "left frame edge is filled");
        assert!(at(2, 4) < 40, "inside the frame is empty");
        assert!(at(6, 6) > 200, "first text line");
        assert!(at(12, 9) < 40, "second line is shorter");
        assert!(at(8, 0) < 10 && at(8, 15) < 10, "nothing above/below the page");
    }

    #[test]
    fn github_mark_shape() {
        let a = rasterize(GITHUB_24, 24.0, 48);
        let at = |x: usize, y: usize| a[y * 48 + x];
        assert!(at(24, 5) > 200, "top of the circle ring");
        assert!(at(24, 20) < 40, "the cat face is cut out");
        assert!(at(2, 2) < 10, "outside the circle");
        let filled = a.iter().filter(|&&v| v > 128).count();
        assert!((600..1400).contains(&filled), "plausible ink coverage: {filled}");
    }
}
