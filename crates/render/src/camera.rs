use mapcraft_geom::{DMat4, DVec2, DVec3, DVec4, WORLD_EXTENT};

/// Which way a viewport looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum View {
    Perspective,
    /// X right, Y up, looking down.
    Top,
    /// Y right, Z up, looking along -X.
    Front,
    /// X right, Z up, looking along +Y.
    Side,
}

impl View {
    pub const ALL: [View; 4] = [View::Perspective, View::Top, View::Front, View::Side];

    pub fn label(self) -> &'static str {
        match self {
            View::Perspective => "3D",
            View::Top => "Top (X/Y)",
            View::Front => "Front (Y/Z)",
            View::Side => "Side (X/Z)",
        }
    }

    /// (right, up, toward viewer) unit axes of a 2D view.
    pub fn axes(self) -> (DVec3, DVec3, DVec3) {
        match self {
            View::Perspective | View::Top => (DVec3::X, DVec3::Y, DVec3::Z),
            View::Front => (DVec3::Y, DVec3::Z, DVec3::X),
            View::Side => (DVec3::X, DVec3::Z, -DVec3::Y),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub view: View,
    /// Eye position of the 3D view.
    pub position: DVec3,
    /// Degrees around Z, 0 looks along +X.
    pub yaw: f64,
    /// Degrees, positive looks up.
    pub pitch: f64,
    pub fov_y: f64,
    /// Center of a 2D view in its (right, up) coordinates.
    pub center: DVec2,
    /// Pixels per unit in a 2D view.
    pub zoom: f64,
}

impl Camera {
    pub fn new(view: View) -> Self {
        Self { view, position: DVec3::new(-384.0, -384.0, 256.0), yaw: 45.0, pitch: -25.0, fov_y: 75.0, center: DVec2::ZERO, zoom: 0.5 }
    }

    pub fn is_2d(&self) -> bool {
        self.view != View::Perspective
    }

    pub fn forward(&self) -> DVec3 {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
    }

    pub fn right(&self) -> DVec3 {
        self.forward().cross(DVec3::Z).normalize_or(DVec3::X)
    }

    pub fn view_proj(&self, size: DVec2) -> DMat4 {
        let aspect = (size.x / size.y.max(1.0)).max(1e-3);
        match self.view {
            View::Perspective => perspective(self.fov_y.to_radians(), aspect, 1.0, WORLD_EXTENT * 4.0) * look_to(self.position, self.forward()),
            _ => {
                let (r, u, t) = self.view.axes();
                let (sx, sy) = (2.0 * self.zoom / size.x.max(1.0), 2.0 * self.zoom / size.y.max(1.0));
                let depth = 1.0 / (WORLD_EXTENT * 8.0);
                // Rows map world to (right, up, toward-viewer) then scale into clip space.
                DMat4::from_cols(
                    DVec4::new(r.x * sx, u.x * sy, -t.x * depth, 0.0),
                    DVec4::new(r.y * sx, u.y * sy, -t.y * depth, 0.0),
                    DVec4::new(r.z * sx, u.z * sy, -t.z * depth, 0.0),
                    DVec4::new(-self.center.x * sx, -self.center.y * sy, 0.5, 1.0),
                )
            }
        }
    }

    /// The ray under a pixel (origin at the top left): origin and unit direction.
    pub fn ray(&self, pixel: DVec2, size: DVec2) -> (DVec3, DVec3) {
        if self.is_2d() {
            let (_, _, t) = self.view.axes();
            (self.plane_point(pixel, size) + t * WORLD_EXTENT * 2.0, -t)
        } else {
            let ndc = DVec2::new(pixel.x / size.x.max(1.0) * 2.0 - 1.0, 1.0 - pixel.y / size.y.max(1.0) * 2.0);
            let inv = self.view_proj(size).inverse();
            let far = inv.project_point3(DVec3::new(ndc.x, ndc.y, 1.0));
            (self.position, (far - self.position).normalize_or(self.forward()))
        }
    }

    /// The world point under a pixel on a 2D view's plane through the origin.
    pub fn plane_point(&self, pixel: DVec2, size: DVec2) -> DVec3 {
        let (r, u, _) = self.view.axes();
        let h = self.center.x + (pixel.x - size.x / 2.0) / self.zoom;
        let v = self.center.y - (pixel.y - size.y / 2.0) / self.zoom;
        r * h + u * v
    }

    /// Screen position of a world point, `None` when it is behind a 3D camera.
    pub fn project(&self, p: DVec3, size: DVec2) -> Option<DVec2> {
        let clip = self.view_proj(size) * p.extend(1.0);
        if clip.w <= 1e-9 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(DVec2::new((ndc.x + 1.0) / 2.0 * size.x, (1.0 - ndc.y) / 2.0 * size.y))
    }

    /// Zooms a 2D view by `factor`, keeping the point under `pixel` fixed.
    pub fn zoom_at(&mut self, pixel: DVec2, size: DVec2, factor: f64) {
        let before = self.plane_point(pixel, size);
        self.zoom = (self.zoom * factor).clamp(1.0 / 256.0, 64.0);
        let after = self.plane_point(pixel, size);
        let (r, u, _) = self.view.axes();
        self.center += DVec2::new((before - after).dot(r), (before - after).dot(u));
    }
}

/// Right-handed view matrix looking along `dir` with Z up.
fn look_to(eye: DVec3, dir: DVec3) -> DMat4 {
    let f = dir.normalize_or(DVec3::X);
    let s = f.cross(DVec3::Z).normalize_or(DVec3::Y);
    let u = s.cross(f);
    DMat4::from_cols(
        DVec4::new(s.x, u.x, -f.x, 0.0),
        DVec4::new(s.y, u.y, -f.y, 0.0),
        DVec4::new(s.z, u.z, -f.z, 0.0),
        DVec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Right-handed perspective projection with wgpu's 0 to 1 depth range.
fn perspective(fov_y: f64, aspect: f64, near: f64, far: f64) -> DMat4 {
    let f = 1.0 / (fov_y / 2.0).tan();
    let r = far / (near - far);
    DMat4::from_cols(DVec4::new(f / aspect, 0.0, 0.0, 0.0), DVec4::new(0.0, f, 0.0, 0.0), DVec4::new(0.0, 0.0, r, -1.0), DVec4::new(0.0, 0.0, r * near, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ortho_round_trip() {
        let size = DVec2::new(800.0, 600.0);
        for view in [View::Top, View::Front, View::Side] {
            let mut cam = Camera::new(view);
            cam.center = DVec2::new(100.0, -50.0);
            cam.zoom = 2.0;
            let p = cam.plane_point(DVec2::new(123.0, 456.0), size);
            let back = cam.project(p, size).unwrap();
            assert!(back.abs_diff_eq(DVec2::new(123.0, 456.0), 1e-6), "{view:?} {back:?}");
            let (origin, dir) = cam.ray(DVec2::new(123.0, 456.0), size);
            assert!((origin + dir * WORLD_EXTENT * 2.0).abs_diff_eq(p, 1e-6));
            let (r, u, t) = view.axes();
            assert!(r.cross(u).abs_diff_eq(t, 1e-12), "{view:?} must be right-handed");
        }
    }

    #[test]
    fn perspective_ray_hits_projected_point() {
        let size = DVec2::new(800.0, 600.0);
        let cam = Camera::new(View::Perspective);
        let target = DVec3::new(10.0, 20.0, 30.0);
        let px = cam.project(target, size).unwrap();
        let (o, d) = cam.ray(px, size);
        let to = (target - o).normalize();
        assert!(d.abs_diff_eq(to, 1e-6));
    }

    #[test]
    fn zoom_keeps_cursor_point() {
        let size = DVec2::new(800.0, 600.0);
        let mut cam = Camera::new(View::Top);
        let px = DVec2::new(600.0, 100.0);
        let before = cam.plane_point(px, size);
        cam.zoom_at(px, size, 2.0);
        assert!(cam.plane_point(px, size).abs_diff_eq(before, 1e-9));
    }
}
