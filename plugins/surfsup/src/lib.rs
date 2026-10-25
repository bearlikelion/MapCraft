//! SurfsUp tools for MapCraft: SURGE surf ramps and bhop platform layouts as generator groups.

mod bhop;
mod ramp;

use glam::DVec3;
use mapcraft::plugin::host;
use mapcraft::plugin::types::{Brush, CommandSpec, Hull, IntegerRange, NumberRange, ParamKind, ParamSpec, Surface, Value, Vec3};

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

const SHAPES: &[&str] = &["Straight", "Left", "Right", "Up", "Down", "Dip", "Arc", "Spiral", "S-Curve"];

struct SurfsUp;

fn spec(id: &str, label: &str, help: &str, kind: ParamKind, initial: Value) -> ParamSpec {
    ParamSpec { id: id.into(), label: label.into(), help: help.into(), kind, initial }
}

fn number(id: &str, label: &str, help: &str, min: f64, max: f64, initial: f64) -> ParamSpec {
    spec(id, label, help, ParamKind::Number(NumberRange { min, max, step: 1.0 }), Value::Number(initial))
}

fn integer(id: &str, label: &str, help: &str, min: i64, max: i64, initial: i64) -> ParamSpec {
    spec(id, label, help, ParamKind::Integer(IntegerRange { min, max }), Value::Integer(initial))
}

fn choice(id: &str, label: &str, help: &str, options: &[&str], initial: &str) -> ParamSpec {
    spec(id, label, help, ParamKind::Choice(options.iter().map(|s| s.to_string()).collect()), Value::Text(initial.into()))
}

fn toggle(id: &str, label: &str, help: &str, initial: bool) -> ParamSpec {
    spec(id, label, help, ParamKind::Toggle, Value::Toggle(initial))
}

fn material(id: &str, label: &str, help: &str) -> ParamSpec {
    spec(id, label, help, ParamKind::Material, Value::Text(String::new()))
}

/// Typed access to the parameters the host sends; it already filled and clamped them.
struct Params(Vec<Param>);

impl Params {
    fn get(&self, id: &str) -> Option<&Value> {
        self.0.iter().find(|p| p.id == id).map(|p| &p.value)
    }

    fn num(&self, id: &str, default: f64) -> f64 {
        match self.get(id) {
            Some(Value::Number(n)) => *n,
            Some(Value::Integer(i)) => *i as f64,
            _ => default,
        }
    }

    fn int(&self, id: &str, default: i64) -> i64 {
        match self.get(id) {
            Some(Value::Integer(i)) => *i,
            Some(Value::Number(n)) => n.round() as i64,
            _ => default,
        }
    }

    fn flag(&self, id: &str) -> bool {
        matches!(self.get(id), Some(Value::Toggle(true)))
    }

    fn text(&self, id: &str) -> &str {
        match self.get(id) {
            Some(Value::Text(s)) => s,
            _ => "",
        }
    }
}

fn v(p: DVec3) -> Vec3 {
    Vec3 { x: p.x, y: p.y, z: p.z }
}

fn ramp_command() -> CommandSpec {
    let d = ramp::Ramp::default();
    CommandSpec {
        id: "surf_ramp".into(),
        label: "Surf Ramp".into(),
        menu: "SurfsUp/Surf Ramp".into(),
        generator: true,
        params: vec![
            choice("shape", "Shape", "The path the ramp follows", SHAPES, "Straight"),
            choice("turn", "Turn", "Which way spirals and S-curves turn first", &["Right", "Left"], "Right"),
            number("size", "Length or radius", "Straight length, or the turn radius at the ramp's center line", 1.0, 65536.0, d.size),
            number("angle", "Angle", "Degrees of turn or pitch for curved shapes", 0.0, 1440.0, d.angle),
            number("rise", "Rise", "Height a spiral climbs over its whole angle", -65536.0, 65536.0, d.rise),
            number("bank", "Bank", "Degrees the ramp leans into turns", -89.0, 89.0, d.bank),
            integer("smoothness", "Segments", "Brushes along a curved ramp", 3, 256, d.smoothness as i64),
            choice("style", "Style", "A solid wedge, or a thin slab under the surf face", &["Wedge", "Thin"], "Wedge"),
            choice("surf", "Surf side", "Which side faces the player", &["Left", "Right", "Both"], "Left"),
            number("width", "Width", "Width of the cross-section", 1.0, 8192.0, d.width),
            number("height", "Height", "Height of the peak", 1.0, 8192.0, d.height),
            number("tip", "Tip", "Height of the surf face's low edge on a wedge", 0.0, 8192.0, d.tip),
            number("thickness", "Thickness", "Slab thickness of a thin ramp", 1.0, 1024.0, d.thickness),
            material("surf_material", "Surf material", "Empty uses the material picked in the browser"),
            material("side_material", "Side material", "Empty uses the material picked in the browser"),
            material("bottom_material", "Bottom material", "Empty uses the side material"),
        ],
    }
}

fn bhop_command() -> CommandSpec {
    let d = bhop::Bhop::default();
    CommandSpec {
        id: "bhop".into(),
        label: "Bhop Platforms".into(),
        menu: "SurfsUp/Bhop Platforms".into(),
        generator: true,
        params: vec![
            choice("pattern", "Pattern", "How the platforms are laid out", &["Linear", "Grid", "Spiral"], "Linear"),
            integer("count", "Platforms", "How many platforms", 1, 4096, d.count as i64),
            number("width", "Width", "Platform size along X", 1.0, 8192.0, d.size.x),
            number("depth", "Depth", "Platform size along Y", 1.0, 8192.0, d.size.y),
            number("height", "Height", "Platform thickness", 1.0, 8192.0, d.size.z),
            number("variation", "Size variation", "Each platform's size changes by up to this fraction", 0.0, 0.9, d.variation),
            integer("seed", "Seed", "Changes the size variation", 1, i64::from(u32::MAX), d.seed as i64),
            number("spacing", "Spacing", "Linear: distance between platforms", 0.0, 65536.0, d.spacing),
            number("rise", "Rise", "Linear: height gained per platform", -8192.0, 8192.0, d.rise),
            number("heading", "Heading", "Linear: direction in degrees from +X", -360.0, 360.0, d.heading),
            number("zigzag", "Zigzag", "Linear: sideways swing, 0 for a straight line", 0.0, 8192.0, d.zigzag),
            integer("columns", "Columns", "Grid: platforms per row", 1, 256, d.columns as i64),
            integer("rows", "Rows", "Grid: number of rows", 1, 256, d.rows as i64),
            number("grid_spacing", "Grid spacing", "Grid: distance between platforms", 0.0, 65536.0, d.grid_spacing),
            number("row_rise", "Row rise", "Grid: height gained per row", -8192.0, 8192.0, d.row_rise),
            toggle("stagger", "Stagger", "Grid: shift every other row by half a step", d.stagger),
            number("radius", "Radius", "Spiral: starting radius", 0.0, 65536.0, d.radius),
            number("radius_per_turn", "Radius per turn", "Spiral: growth per full turn", -65536.0, 65536.0, d.radius_per_turn),
            number("angle_step", "Angle step", "Spiral: degrees between platforms", 1.0, 360.0, d.angle_step),
            number("rise_per_turn", "Rise per turn", "Spiral: height gained per full turn", -65536.0, 65536.0, d.rise_per_turn),
            toggle("clockwise", "Clockwise", "Spiral: turn clockwise seen from above", d.clockwise),
            material("material", "Material", "Empty uses the material picked in the browser"),
        ],
    }
}

/// The value paired with the chosen option; `options` and `values` have the same length.
fn pick<T: Copy>(options: &[&str], values: &[T], chosen: &str) -> T {
    values[options.iter().position(|o| *o == chosen).unwrap_or(0)]
}

fn surf_ramp(p: &Params) -> Result<Output, String> {
    use ramp::{Shape, Style, Surf, Turn};
    let r = ramp::Ramp {
        shape: pick(
            SHAPES,
            &[Shape::Straight, Shape::Left, Shape::Right, Shape::Up, Shape::Down, Shape::Dip, Shape::Arc, Shape::Spiral, Shape::SCurve],
            p.text("shape"),
        ),
        turn: pick(&["Right", "Left"], &[Turn::Right, Turn::Left], p.text("turn")),
        size: p.num("size", 1024.0),
        angle: p.num("angle", 90.0),
        rise: p.num("rise", 512.0),
        bank: p.num("bank", 0.0),
        smoothness: p.int("smoothness", 16).clamp(3, 256) as usize,
        style: pick(&["Wedge", "Thin"], &[Style::Wedge, Style::Thin], p.text("style")),
        surf: pick(&["Left", "Right", "Both"], &[Surf::Left, Surf::Right, Surf::Both], p.text("surf")),
        width: p.num("width", 384.0),
        height: p.num("height", 544.0),
        tip: p.num("tip", 64.0),
        thickness: p.num("thickness", 64.0),
    };
    let run = if r.surf == Surf::Both { r.width } else { r.width * 0.5 };
    let face = ramp::slope_angle(run, r.height, if r.style == Style::Thin { 0.0 } else { r.tip });
    if face < ramp::SURF_MIN_ANGLE {
        host::log(&format!("The surf face is {face:.1} degrees; under {} degrees is walkable floor, not surfable.", ramp::SURF_MIN_ANGLE));
    }
    let (surf, side) = (p.text("surf_material"), p.text("side_material"));
    let bottom = if p.text("bottom_material").is_empty() { side } else { p.text("bottom_material") };
    let brushes = ramp::build(&r)?
        .into_iter()
        .map(|piece| {
            let surfaces = piece
                .faces
                .iter()
                .map(|(n, slot)| Surface { normal: v(*n), material: if *slot == ramp::Slot::Surf { surf } else { bottom }.to_string() })
                .collect();
            Brush::Hull(Hull { points: piece.points.into_iter().map(v).collect(), material: side.to_string(), surfaces })
        })
        .collect();
    Ok(Output { brushes, entities: Vec::new() })
}

fn bhop(p: &Params) -> Output {
    use bhop::Pattern;
    let b = bhop::Bhop {
        pattern: pick(&["Linear", "Grid", "Spiral"], &[Pattern::Linear, Pattern::Grid, Pattern::Spiral], p.text("pattern")),
        count: p.int("count", 30).clamp(1, 4096) as usize,
        size: DVec3::new(p.num("width", 192.0), p.num("depth", 192.0), p.num("height", 16.0)),
        variation: p.num("variation", 0.0),
        seed: p.int("seed", 1).max(1) as u64,
        spacing: p.num("spacing", 320.0),
        rise: p.num("rise", 64.0),
        heading: p.num("heading", 0.0),
        zigzag: p.num("zigzag", 0.0),
        columns: p.int("columns", 5).clamp(1, 256) as usize,
        rows: p.int("rows", 6).clamp(1, 256) as usize,
        grid_spacing: p.num("grid_spacing", 384.0),
        row_rise: p.num("row_rise", 128.0),
        stagger: p.flag("stagger"),
        radius: p.num("radius", 192.0),
        radius_per_turn: p.num("radius_per_turn", 64.0),
        angle_step: p.num("angle_step", 45.0),
        rise_per_turn: p.num("rise_per_turn", 384.0),
        clockwise: p.flag("clockwise"),
    };
    let material = p.text("material").to_string();
    let brushes = bhop::platforms(&b)
        .into_iter()
        .map(|(c, s)| {
            let h = s / 2.0;
            let points = (0..8)
                .map(|i| v(c + DVec3::new(if i & 1 == 0 { -h.x } else { h.x }, if i & 2 == 0 { -h.y } else { h.y }, if i & 4 == 0 { -h.z } else { h.z })))
                .collect();
            Brush::Hull(Hull { points, material: material.clone(), surfaces: Vec::new() })
        })
        .collect();
    Output { brushes, entities: Vec::new() }
}

impl Guest for SurfsUp {
    fn manifest() -> Manifest {
        Manifest {
            id: "surfsup".into(),
            name: "SurfsUp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: vec![ramp_command(), bhop_command()],
            materials: false,
        }
    }

    fn run_command(id: String, params: Vec<Param>, _context: Context) -> Result<Output, String> {
        let p = Params(params);
        match id.as_str() {
            "surf_ramp" => surf_ramp(&p),
            "bhop" => Ok(bhop(&p)),
            other => Err(format!("unknown command {other}")),
        }
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        Ok(Vec::new())
    }
}

export!(SurfsUp);
