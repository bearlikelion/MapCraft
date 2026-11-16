//! Imports of real zones, when an EverQuest install is on this machine, and the format readers.

use crate::mapcraft::plugin::types::Brush;
use crate::model;
use crate::pfs::Archive;
use glam::{DVec2, DVec3};

use crate::mapcraft::plugin::types::TexAxis;
use crate::zone::{import, tex_axes};

const EQ: &str = "/mnt/Storage/Everquest/everquest_rof2";

#[test]
fn imports_real_zones() {
    for zone in ["gfaydark", "qeynos2", "postorms"] {
        let Ok(data) = std::fs::read(format!("{EQ}/{zone}.s3d")) else { return };
        let started = std::time::Instant::now();
        let imported = import(&format!("{zone}.s3d"), &data).unwrap();
        let brushes: usize = imported.output.groups.iter().map(|g| g.brushes.len() + g.entities.iter().map(|e| e.brushes.len()).sum::<usize>()).sum();
        let world = &imported.output.groups[0].brushes;
        let textured = world.iter().filter(|b| matches!(b, Brush::Faces(f) if f[0].uv.is_some())).count();
        let mut flat: std::collections::BTreeMap<&str, usize> = Default::default();
        for b in world {
            if let Brush::Faces(f) = b
                && f[0].uv.is_none()
            {
                *flat.entry(f[0].material.as_str()).or_default() += 1;
            }
        }
        println!(
            "{zone}: {brushes} brushes, {textured} of {} aligned, {} materials in {:?}; unaligned {flat:?}",
            world.len(),
            imported.output.materials.len(),
            started.elapsed()
        );
        for line in &imported.log {
            println!("  {line}");
        }
        assert!(brushes > 1000);
        assert!(textured * 10 > world.len() * 9);
    }
}

#[test]
fn loads_object_models() {
    let Ok(data) = std::fs::read(format!("{EQ}/gfaydark_obj.s3d")) else { return };
    let imported = import("gfaydark.s3d", &std::fs::read(format!("{EQ}/gfaydark.s3d")).unwrap()).unwrap();
    let paths: std::collections::BTreeSet<String> =
        imported.output.groups.iter().flat_map(|g| &g.entities).filter_map(|e| e.properties.iter().find(|p| p.0 == "model").map(|p| p.1.clone())).collect();
    let mut loaded = 0;
    for path in &paths {
        let (archive, actor) = model::split(path).unwrap();
        match model::load(archive, &data, actor) {
            Ok(m) => {
                loaded += 1;
                assert!(!m.parts.is_empty(), "{path}");
            }
            Err(e) => println!("{path}: {e}"),
        }
    }
    println!("{loaded} of {} object models", paths.len());
    assert!(loaded * 10 >= paths.len() * 9);
}

#[test]
fn rejects_broken_archives() {
    assert!(Archive::open(b"").is_err());
    assert!(Archive::open(b"\x10\0\0\0PFS \x00\x00\x02\x00\xff\xff\xff\xff").is_err());
    let mut fake = b"\x0c\0\0\0PFS \0\0\x02\0".to_vec();
    fake.extend_from_slice(&1u32.to_le_bytes());
    fake.extend_from_slice(&0x6158_0ac9u32.to_le_bytes());
    fake.extend_from_slice(&0u32.to_le_bytes());
    fake.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(Archive::open(&fake).is_err());
    assert!(import("x.s3d", &fake).is_err());
}

#[test]
fn model_paths() {
    assert_eq!(model::split("gfaydark_obj.s3d/tree1.actordef"), Some(("gfaydark_obj.s3d", "tree1")));
    assert_eq!(model::split("tree1.actordef"), None);
    assert_eq!(model::split("a.eqg/tree1.actordef"), None);
}

#[test]
fn texture_axes_reproduce_uvs() {
    let p = [DVec3::new(0.0, 0.0, 5.0), DVec3::new(10.0, 0.0, 5.0), DVec3::new(0.0, 20.0, 5.0)];
    let texel = |(u, v): &(TexAxis, TexAxis), q: DVec3| {
        let at = |a: &TexAxis| q.dot(DVec3::new(a.axis.x, a.axis.y, a.axis.z)) / a.scale + a.offset;
        DVec2::new(at(u), at(v))
    };
    for uv in [
        [DVec2::new(1.0, 2.0), DVec2::new(65.0, 2.0), DVec2::new(1.0, 130.0)],
        [DVec2::new(3.0, 7.0), DVec2::new(9.0, 7.0), DVec2::new(5.0, 7.0)],
        [DVec2::splat(4.0); 3],
    ] {
        let axes = tex_axes(p, uv).unwrap();
        for (q, want) in p.iter().zip(uv) {
            assert!(texel(&axes, *q).distance(want) < 0.01, "{uv:?}");
        }
    }
    assert!(tex_axes([DVec3::ZERO, DVec3::X, DVec3::X * 2.0], [DVec2::ZERO; 3]).is_none());
}

/// Placed objects as the importer turns them sink into walls less than turned the other ways EverQuest's heading could mean.
#[test]
fn objects_face_the_right_way() {
    use std::collections::HashMap;

    use glam::DMat3;

    let (Ok(data), Ok(obj)) = (std::fs::read(format!("{EQ}/qeynos2.s3d")), std::fs::read(format!("{EQ}/qeynos2_obj.s3d"))) else { return };
    let archive = Archive::open(&data).unwrap();
    let w = archive.read("qeynos2.wld").unwrap();
    let wld = crate::wld::Wld::parse(&w).unwrap();
    let mut tris: Vec<[DVec3; 3]> = Vec::new();
    for f in wld.of_kind(crate::wld::MESH) {
        let m = wld.mesh(f).unwrap();
        tris.extend(m.triangles.iter().map(|t| [t.corners[0], t.corners[2], t.corners[1]].map(|i| DVec3::from(m.positions[i].map(f64::from)))));
    }
    let cell = |p: DVec3| (p / 8.0).floor().as_i64vec3().to_array();
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        let (a, b) = (cell(t[0].min(t[1]).min(t[2])), cell(t[0].max(t[1]).max(t[2])));
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    grid.entry([x, y, z]).or_default().push(i);
                }
            }
        }
    }
    // Behind the nearest surface it lies over, within a few units.
    let inside = |q: DVec3| {
        let c = cell(q);
        let near = (0..27).flat_map(|k| grid.get(&[c[0] + k % 3 - 1, c[1] + k / 3 % 3 - 1, c[2] + k / 9 - 1])).flatten();
        let hit = near
            .filter_map(|&i| {
                let t = tris[i];
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                let d = (q - t[0]).dot(n);
                let p = q - n * d;
                (0..3).all(|k| (t[(k + 1) % 3] - t[k]).cross(p - t[k]).dot(n) >= 0.0).then_some(d)
            })
            .min_by(|a, b| a.abs().total_cmp(&b.abs()));
        hit.is_some_and(|d| d.abs() < 4.0 && d < -0.25)
    };
    let imported = import("qeynos2.s3d", &data).unwrap();
    let mut models: HashMap<String, Vec<DVec3>> = HashMap::new();
    let mut sunk = [0usize; 3];
    for e in imported.output.groups.iter().flat_map(|g| &g.entities).filter(|e| e.classname == "prop_static") {
        let get = |k: &str| e.properties.iter().find(|p| p.0 == k).map(|p| p.1.clone()).unwrap_or_default();
        let nums = |k: &str| get(k).split(' ').map(|v| v.parse::<f64>().unwrap()).collect::<Vec<_>>();
        let (origin, angles, scale) = (nums("origin"), nums("angles"), nums("scale")[0]);
        let path = get("model");
        let (arch, actor) = model::split(&path).unwrap();
        let points = models.entry(path.clone()).or_insert_with(|| {
            model::load(arch, &obj, actor)
                .map(|m| m.parts.iter().flat_map(|p| p.positions.iter().map(|v| DVec3::new(v.x, v.y, v.z))).collect())
                .unwrap_or_default()
        });
        let pitch = DMat3::from_rotation_y(angles[0].to_radians());
        for (k, yaw) in [angles[1], angles[1] + 180.0, -angles[1]].into_iter().enumerate() {
            let turn = DMat3::from_rotation_z(yaw.to_radians()) * pitch;
            sunk[k] += points.iter().filter(|v| inside(DVec3::new(origin[0], origin[1], origin[2]) + turn * (**v * scale))).count();
        }
    }
    assert!(sunk[0] * 3 < sunk[1] * 2 && sunk[0] * 3 < sunk[2] * 2, "{sunk:?}");
}
