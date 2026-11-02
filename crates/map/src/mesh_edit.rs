//! A prop's changes to the meshes of its model, kept in `mesh.<name>.<field>` keys.

use std::collections::BTreeMap;

use mapcraft_geom::{DMat3, DMat4, DQuat, DVec3, EulerRot};

use crate::Entity;

/// Prefix of the keys that change one mesh of a prop: `mesh.<name>.origin`, `.angles`, `.scale`, `.hidden` and `.collision`.
pub const MESH_KEY: &str = "mesh.";

/// One mesh's changes on a prop, relative to where its model puts it.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshEdit {
    pub origin: DVec3,
    /// Pitch, yaw and roll in degrees, as an entity's `angles`.
    pub angles: [f64; 3],
    pub scale: f64,
    /// Neither drawn nor collided with, together with the meshes under it.
    pub hidden: bool,
    /// `none`, `convex` or `trimesh` in place of the prop's own `collision`.
    pub collision: Option<String>,
}

impl Default for MeshEdit {
    fn default() -> Self {
        Self { origin: DVec3::ZERO, angles: [0.0; 3], scale: 1.0, hidden: false, collision: None }
    }
}

/// Hammer's pitch, yaw and roll: yaw about Z, then pitch about Y, then roll about X.
pub fn hammer_turn(pitch: f64, yaw: f64, roll: f64) -> DQuat {
    DQuat::from_rotation_z(yaw.to_radians()) * DQuat::from_rotation_y(pitch.to_radians()) * DQuat::from_rotation_x(roll.to_radians())
}

impl MeshEdit {
    /// The change in model space, turning and scaling about `pivot`, the mesh's own origin.
    pub fn matrix(&self, pivot: DVec3) -> DMat4 {
        let [pitch, yaw, roll] = self.angles;
        DMat4::from_translation(pivot + self.origin)
            * DMat4::from_scale_rotation_translation(DVec3::splat(self.scale), hammer_turn(pitch, yaw, roll), DVec3::ZERO)
            * DMat4::from_translation(-pivot)
    }

    /// The edit whose [`MeshEdit::matrix`] about `pivot` is `m`, keeping this one's visibility and collision;
    /// `None` when `m` mirrors or flattens. A stretch becomes its average scale.
    pub fn with_matrix(&self, m: &DMat4, pivot: DVec3) -> Option<Self> {
        let linear = DMat3::from_mat4(*m);
        let det = linear.determinant();
        if !det.is_finite() || det <= 1e-12 {
            return None;
        }
        let scale = det.cbrt();
        let (yaw, pitch, roll) = DQuat::from_mat3(&(linear * (1.0 / scale))).normalize().to_euler(EulerRot::ZYX);
        let tidy = |d: f64| {
            let r = (d * 1e4).round() / 1e4;
            if r == 0.0 { 0.0 } else { r }
        };
        let angles = [tidy(pitch.to_degrees()), tidy(yaw.to_degrees()), tidy(roll.to_degrees())];
        let origin = m.w_axis.truncate() - pivot + linear * pivot;
        Some(Self { origin, angles, scale, ..self.clone() })
    }
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>().unwrap_or_default()
}

/// The changes a prop's `mesh.` keys make, by mesh name.
pub fn mesh_edits(e: &Entity) -> BTreeMap<String, MeshEdit> {
    let mut out: BTreeMap<String, MeshEdit> = BTreeMap::new();
    for (key, value) in &e.properties {
        let Some((name, field)) = key.strip_prefix(MESH_KEY).and_then(|k| k.rsplit_once('.')) else { continue };
        let edit = out.entry(name.to_string()).or_default();
        match (field, numbers(value).as_slice()) {
            ("origin", &[x, y, z]) => edit.origin = DVec3::new(x, y, z),
            ("angles", &[p, y, r]) => edit.angles = [p, y, r],
            ("scale", &[s]) if s > 0.0 => edit.scale = s,
            ("hidden", _) => edit.hidden = !matches!(value.trim(), "" | "0" | "false"),
            ("collision", _) if !value.trim().is_empty() => edit.collision = Some(value.trim().to_ascii_lowercase()),
            _ => {}
        }
    }
    out
}

fn num(v: f64) -> String {
    let r = (v * 1e4).round() / 1e4 + 0.0;
    if r == r.trunc() { format!("{r:.0}") } else { format!("{r}") }
}

/// Writes `edit` to `e`'s keys for mesh `name`, leaving out the fields that change nothing.
pub fn set_mesh_edit(e: &mut Entity, name: &str, edit: &MeshEdit) {
    let key = |field: &str| format!("{MESH_KEY}{name}.{field}");
    let mut put = |field: &str, value: Option<String>| match value {
        Some(v) => e.set(key(field), v),
        None => {
            e.remove(&key(field));
        }
    };
    let o = edit.origin;
    let [p, y, r] = edit.angles;
    put("origin", (o.abs().max_element() >= 5e-5).then(|| format!("{} {} {}", num(o.x), num(o.y), num(o.z))));
    put("angles", edit.angles.iter().any(|a| a.abs() >= 5e-5).then(|| format!("{} {} {}", num(p), num(y), num(r))));
    put("scale", ((edit.scale - 1.0).abs() >= 5e-5).then(|| num(edit.scale)));
    put("hidden", edit.hidden.then(|| "1".to_string()));
    put("collision", edit.collision.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_read_write_and_round_trip_through_matrices() {
        let mut e = Entity::new("prop_static");
        e.set("mesh.a.b.collision", "Trimesh");
        e.set("mesh.a.b.scale", "-1");
        let edits = mesh_edits(&e);
        assert_eq!(
            edits.get("a.b"),
            Some(&MeshEdit { collision: Some("trimesh".into()), ..MeshEdit::default() }),
            "names may hold dots, bad scales are ignored"
        );

        let edit = MeshEdit { origin: DVec3::new(1.0, 2.0, 3.0), angles: [10.0, 20.0, 30.0], scale: 2.0, ..MeshEdit::default() };
        let pivot = DVec3::new(5.0, -3.0, 8.0);
        let back = MeshEdit::default().with_matrix(&edit.matrix(pivot), pivot).unwrap();
        assert!(back.origin.abs_diff_eq(edit.origin, 1e-9) && (back.scale - 2.0).abs() < 1e-9, "{back:?}");
        assert!(back.angles.iter().zip(edit.angles).all(|(a, b)| (a - b).abs() < 1e-3), "{back:?}");
        assert!(MeshEdit::default().with_matrix(&DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)), pivot).is_none(), "a mirror is refused");

        set_mesh_edit(&mut e, "a.b", &MeshEdit { origin: DVec3::new(0.0, 0.0, 16.0), hidden: true, ..MeshEdit::default() });
        assert_eq!(e.get("mesh.a.b.origin"), Some("0 0 16"));
        assert_eq!((e.get("mesh.a.b.hidden"), e.get("mesh.a.b.collision"), e.get("mesh.a.b.scale")), (Some("1"), None, None));
    }
}
