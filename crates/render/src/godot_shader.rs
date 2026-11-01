//! Godot shading language to GLSL, so the 3D views can preview `.gdshader` files.

/// The Godot shader types the preview draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind {
    /// Drawn on the faces of one material.
    Spatial,
    /// Drawn behind the map in place of the sky.
    Sky,
}

/// Source the preview refuses, so hostile text cannot stall the frame.
const MAX_SOURCE: usize = 256 * 1024;

/// Matches the start of `Globals` in shaders.wgsl; the lights after the first are left off.
const GLOBALS: &str = "layout(set = 0, binding = 0) uniform Globals {
	mat4 view_proj; vec4 light_dir; vec4 ambient; vec4 ground; vec4 eye; mat4 inv_view_proj;
	vec4 sky_top; vec4 sky_horizon; vec4 sky_ground; vec4 fog_color; vec4 fog; vec4 tonemap;
	vec4 adjustments; vec4 sky_shape; vec4 minlight; vec4 paint; vec4 light0_pos; vec4 light0_color;
} mc;
layout(location = 0) out vec4 mc_out;
const float PI = 3.14159265359; const float TAU = 6.28318530718; const float E = 2.71828182846;
float TIME;
vec3 mc_godot(vec3 v) { return vec3(v.x, v.z, -v.y); }
vec3 mc_linear(vec3 c) { return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(vec3(0.04045), c)); }
";

const SPATIAL: &str = "layout(location = 0) in vec3 mc_normal; layout(location = 1) in vec2 mc_uv; layout(location = 2) in vec4 mc_tint;
layout(location = 3) in vec3 mc_world; layout(location = 4) in vec4 mc_tangent;
vec2 UV; vec2 UV2; vec4 COLOR; vec3 VERTEX; vec3 NORMAL; vec3 TANGENT; vec3 BINORMAL; vec3 VIEW; vec4 FRAGCOORD;
bool FRONT_FACING; vec3 CAMERA_POSITION_WORLD; vec3 CAMERA_DIRECTION_WORLD; vec3 NODE_POSITION_WORLD; vec2 SCREEN_UV;
mat4 MODEL_MATRIX; mat4 VIEW_MATRIX; mat4 INV_VIEW_MATRIX; mat4 PROJECTION_MATRIX; mat4 INV_PROJECTION_MATRIX;
vec3 ALBEDO; float ALPHA; float ALPHA_SCISSOR_THRESHOLD; float ROUGHNESS; float METALLIC; float SPECULAR; vec3 EMISSION;
vec3 NORMAL_MAP; float NORMAL_MAP_DEPTH; float AO; float AO_LIGHT_AFFECT; float RIM; float RIM_TINT; float CLEARCOAT;
float CLEARCOAT_ROUGHNESS; float ANISOTROPY; vec2 ANISOTROPY_FLOW; float SSS_STRENGTH; vec3 BACKLIGHT; vec3 BENT_NORMAL_MAP;
";

const SKY: &str = "layout(location = 0) in vec2 mc_ndc;
vec3 EYEDIR; vec2 SCREEN_UV; vec2 SKY_COORDS; vec3 POSITION; vec3 COLOR; float ALPHA; vec4 FOG; vec4 FRAGCOORD;
vec4 HALF_RES_COLOR; vec4 QUARTER_RES_COLOR; bool AT_CUBEMAP_PASS; bool AT_HALF_RES_PASS; bool AT_QUARTER_RES_PASS;
bool LIGHT0_ENABLED; vec3 LIGHT0_DIRECTION; float LIGHT0_ENERGY; vec3 LIGHT0_COLOR; float LIGHT0_SIZE;
bool LIGHT1_ENABLED; vec3 LIGHT1_DIRECTION; float LIGHT1_ENERGY; vec3 LIGHT1_COLOR; float LIGHT1_SIZE;
bool LIGHT2_ENABLED; vec3 LIGHT2_DIRECTION; float LIGHT2_ENERGY; vec3 LIGHT2_COLOR; float LIGHT2_SIZE;
bool LIGHT3_ENABLED; vec3 LIGHT3_DIRECTION; float LIGHT3_ENERGY; vec3 LIGHT3_COLOR; float LIGHT3_SIZE;
";

const SPATIAL_MAIN: &str = "	TIME = mc.paint.y; UV = mc_uv; UV2 = mc_uv; COLOR = mc_tint; FRAGCOORD = gl_FragCoord; FRONT_FACING = gl_FrontFacing; SCREEN_UV = gl_FragCoord.xy / mc.paint.zw;
	VERTEX = mc_godot(mc_world - mc.eye.xyz) / 39.37; NORMAL = normalize(mc_godot(mc_normal)); VIEW = normalize(-VERTEX);
	TANGENT = normalize(mc_godot(mc_tangent.xyz)); BINORMAL = cross(NORMAL, TANGENT) * mc_tangent.w;
	CAMERA_POSITION_WORLD = mc_godot(mc.eye.xyz) / 39.37; CAMERA_DIRECTION_WORLD = -VIEW;
	MODEL_MATRIX = mat4(1.0); VIEW_MATRIX = mat4(1.0); INV_VIEW_MATRIX = mat4(1.0); PROJECTION_MATRIX = mat4(1.0); INV_PROJECTION_MATRIX = mat4(1.0);
	ALBEDO = vec3(1.0); ALPHA = 1.0; ROUGHNESS = 1.0; SPECULAR = 0.5; AO = 1.0; NORMAL_MAP = vec3(0.5, 0.5, 1.0); NORMAL_MAP_DEPTH = 1.0;
";

const SKY_MAIN: &str = "	TIME = mc.paint.y; FRAGCOORD = gl_FragCoord; ALPHA = 1.0;
	vec4 mc_p = mc.inv_view_proj * vec4(mc_ndc, 0.5, 1.0);
	EYEDIR = normalize(mc_godot(mc_p.xyz / mc_p.w - mc.eye.xyz));
	SCREEN_UV = mc_ndc * vec2(0.5, -0.5) + 0.5; POSITION = mc_godot(mc.eye.xyz) / 39.37;
	SKY_COORDS = vec2(atan(EYEDIR.x, -EYEDIR.z) / TAU + 0.5, acos(clamp(EYEDIR.y, -1.0, 1.0)) / PI);
	bool mc_sun = mc.light0_pos.w == 0.0 && dot(mc.light0_pos.xyz, mc.light0_pos.xyz) > 0.0;
	LIGHT0_ENABLED = true;
	LIGHT0_DIRECTION = normalize(mc_godot(mc_sun ? mc.light0_pos.xyz : mc.light_dir.xyz)); LIGHT0_COLOR = mc_sun ? mc.light0_color.rgb : vec3(1.0);
	LIGHT0_ENERGY = 1.0; LIGHT0_SIZE = 0.02;
";

/// Texture uniforms a preview can bind.
pub const MAX_TEXTURES: usize = 8;

/// What a texture uniform samples; the preview has no textures assigned, so each gets Godot's default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureSource {
    White,
    Black,
    Transparent,
    Normal,
    /// The frame drawn so far, for `hint_screen_texture`.
    Screen,
}

/// A Godot shader as GLSL 4.50 fragment source, with the number of lines put before the user's first.
pub struct Translated {
    pub kind: ShaderKind,
    pub glsl: String,
    pub offset: usize,
    /// Bound in group 1 as texture `2 * i` and sampler `2 * i + 1`.
    pub textures: Vec<TextureSource>,
}

/// Comments as spaces, keeping newlines so lines still match the source.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                while chars.peek().is_some_and(|c| *c != '\n') {
                    chars.next();
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                    }
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Top-level statements, each ending at a `;` or a closing brace outside any braces.
fn statements(source: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut depth) = (0usize, 0usize);
    for (i, b) in source.bytes().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && (b == b';' || b == b'}') {
            out.push(source.get(start..=i).unwrap_or_default());
            start = i + 1;
        }
    }
    out.push(source.get(start..).unwrap_or_default());
    out
}

fn line_of(source: &str, statement: &str) -> usize {
    let at = (statement.as_ptr() as usize).saturating_sub(source.as_ptr() as usize);
    let leading = statement.len() - statement.trim_start().len();
    source.get(..at + leading).map_or(1, |s| s.matches('\n').count() + 1)
}

fn has_function(body: &str, name: &str) -> bool {
    let words: Vec<&str> = body.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    words.windows(2).any(|w| matches!(w, ["void", n] if *n == name))
}

/// Translates Godot shader source; the error names the line of anything the preview cannot draw.
pub fn translate(source: &str) -> Result<Translated, String> {
    if source.len() > MAX_SOURCE {
        return Err("Shader source exceeds 256 KiB".into());
    }
    let stripped = strip_comments(source);
    let mut kind = None;
    let mut body = String::with_capacity(stripped.len());
    let mut colors = Vec::new();
    let mut textures = Vec::new();
    let mut render_modes: Vec<String> = Vec::new();
    let mut texture_decls = String::new();
    for statement in statements(&stripped) {
        let newlines = "\n".repeat(statement.matches('\n').count());
        let words: Vec<&str> = statement.split_whitespace().collect();
        let line = line_of(&stripped, statement);
        match words.first().map(|w| w.trim_end_matches(';')) {
            Some("shader_type") => {
                kind = Some(match words.get(1).map(|w| w.trim_end_matches(';')) {
                    Some("spatial") => ShaderKind::Spatial,
                    Some("sky") => ShaderKind::Sky,
                    Some(other) => return Err(format!("Line {line}: the preview draws spatial and sky shaders, not {other}")),
                    None => return Err(format!("Line {line}: shader_type needs a type")),
                });
                body.push_str(&newlines);
            }
            Some("render_mode") => {
                render_modes.extend(statement.trim().trim_end_matches(';').split([',', ' ', '\t', '\n']).filter(|w| !w.is_empty()).map(String::from));
                body.push_str(&newlines);
            }
            Some("group_uniforms" | "subgroup_uniforms") => body.push_str(&newlines),
            Some("uniform" | "global" | "instance") => {
                let text = statement.trim().trim_end_matches(';');
                let text = text.trim_start_matches("global").trim_start_matches("instance").trim_start().trim_start_matches("uniform");
                let (declaration, default) = text.split_once('=').map_or((text, None), |(d, v)| (d, Some(v.trim())));
                let (declaration, hints) = declaration.split_once(':').unwrap_or((declaration, ""));
                let parts: Vec<&str> = declaration.split_whitespace().collect();
                let (Some(name), Some(ty)) = (parts.last(), parts.len().checked_sub(2).and_then(|i| parts.get(i))) else {
                    return Err(format!("Line {line}: could not read this uniform"));
                };
                if *ty == "sampler2D" {
                    if textures.len() >= MAX_TEXTURES {
                        return Err(format!("Line {line}: the preview binds at most {MAX_TEXTURES} textures"));
                    }
                    let binding = textures.len() * 2;
                    textures.push(if hints.contains("hint_screen_texture") {
                        TextureSource::Screen
                    } else if hints.contains("hint_normal") {
                        TextureSource::Normal
                    } else if hints.contains("hint_default_black") || hints.contains("hint_depth_texture") {
                        TextureSource::Black
                    } else if hints.contains("hint_default_transparent") {
                        TextureSource::Transparent
                    } else {
                        TextureSource::White
                    });
                    texture_decls.push_str(&format!(
                        "layout(set = 1, binding = {binding}) uniform texture2D mc_tex_{name}; layout(set = 1, binding = {}) uniform sampler mc_smp_{name};\n#define {name} sampler2D(mc_tex_{name}, mc_smp_{name})\n",
                        binding + 1
                    ));
                    body.push_str(&newlines);
                    continue;
                }
                if ty.starts_with("sampler") || ty.starts_with("isampler") || ty.starts_with("usampler") {
                    return Err(format!("Line {line}: the preview supports sampler2D textures, not {ty} {name}"));
                }
                if hints.contains("source_color") && matches!(*ty, "vec3" | "vec4") {
                    colors.push((*name).to_string());
                }
                match default {
                    Some(value) => body.push_str(&format!("{ty} {name} = {value};")),
                    None => body.push_str(&format!("{ty} {name};")),
                }
                body.push_str(&newlines);
            }
            Some("varying") => {
                let rest: Vec<&str> = words.iter().skip(1).copied().filter(|w| !matches!(*w, "flat" | "smooth")).collect();
                body.push_str(&rest.join(" "));
                body.push_str(&newlines);
            }
            _ => body.push_str(statement),
        }
    }
    let kind = kind.ok_or("The shader needs a shader_type line, such as shader_type spatial;")?;
    let (inputs, setup, entry) = match kind {
        ShaderKind::Spatial => (SPATIAL, SPATIAL_MAIN, "fragment"),
        ShaderKind::Sky => (SKY, SKY_MAIN, "sky"),
    };
    let prelude = format!("#version 450\n{GLOBALS}{inputs}{texture_decls}");
    let mut main = String::from("void main() {\n");
    main.push_str(setup);
    for name in &colors {
        main.push_str(&format!("\t{name}.rgb = mc_linear({name}.rgb);\n"));
    }
    if has_function(&body, entry) {
        // Godot's lower resolution sky passes run first and hand their result to the full pass.
        for (mode, flag, color) in
            [("use_quarter_res_pass", "AT_QUARTER_RES_PASS", "QUARTER_RES_COLOR"), ("use_half_res_pass", "AT_HALF_RES_PASS", "HALF_RES_COLOR")]
        {
            if kind == ShaderKind::Sky && render_modes.iter().any(|m| m == mode) {
                main.push_str(&format!("\t{flag} = true;\n\t{entry}();\n\t{color} = vec4(COLOR, ALPHA);\n\t{flag} = false; COLOR = vec3(0.0); ALPHA = 1.0;\n"));
            }
        }
        main.push_str(&format!("\t{entry}();\n"));
    }
    main.push_str(match kind {
        ShaderKind::Spatial => {
            "\tfloat mc_shade = 0.6 + 0.4 * abs(dot(normalize(mc_normal), mc.light_dir.xyz));\n\tmc_out = vec4(ALBEDO * mc_shade + EMISSION, ALPHA);\n}\n"
        }
        ShaderKind::Sky => "\tmc_out = vec4(COLOR, 1.0);\n}\n",
    });
    let offset = prelude.matches('\n').count();
    Ok(Translated { kind, glsl: format!("{prelude}{body}\n{main}"), offset, textures })
}

/// Parses and validates the translated source, with errors on the user's own lines.
pub fn compile(source: &str) -> Result<(ShaderKind, Vec<TextureSource>, naga::Module), String> {
    let t = translate(source)?;
    let at = |line: usize| match line.checked_sub(t.offset) {
        Some(l) if l > 0 && l <= source.lines().count() => format!("Line {l}: "),
        _ => String::new(),
    };
    let module = naga::front::glsl::Frontend::default().parse(&naga::front::glsl::Options::from(naga::ShaderStage::Fragment), &t.glsl).map_err(|errors| {
        let lines: Vec<String> =
            errors.errors.iter().take(8).map(|e| format!("{}{}", e.location(&t.glsl).map_or(String::new(), |l| at(l.line_number as usize)), e.kind)).collect();
        lines.join("\n")
    })?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).map_err(|e| {
        let line = e.spans().next().map_or(String::new(), |(span, _)| at(span.location(&t.glsl).line_number as usize));
        format!("{line}{}", e.as_inner())
    })?;
    Ok((t.kind, t.textures, module))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sky_shader_with_hinted_multiline_uniforms_compiles() {
        let source = "shader_type sky;\n// note\ngroup_uniforms g;\ngroup_uniforms;\nuniform vec3 a : source_color =\n\tvec3(0.1, 0.2, 0.3);\nuniform float s : hint_range(0.0, 1.0) = 0.5;\n\nfloat h(float p) { p = fract(p * 0.1); return p; }\nvoid sky() {\n\tCOLOR = a * h(TIME) + EYEDIR * s;\n}\n";
        assert_eq!(compile(source).map(|(k, _, _)| k), Ok(ShaderKind::Sky));
    }

    #[test]
    fn a_spatial_shader_compiles_and_errors_name_the_users_line() {
        assert!(compile("shader_type spatial;\nrender_mode unshaded;\nvoid fragment() { ALBEDO = vec3(UV, sin(TIME)); }\n").is_ok());
        let error = compile("shader_type spatial;\n\nvoid fragment() {\n\tALBEDO = nope;\n}\n").map(|_| ()).unwrap_err();
        assert!(error.starts_with("Line 4: "), "{error}");
    }

    #[test]
    fn texture_uniforms_get_godot_defaults_and_keep_line_numbers() {
        let source = "shader_type spatial;\nuniform sampler2D screen_texture : hint_screen_texture, filter_linear_mipmap;\nuniform sampler2D n : hint_normal;\nuniform sampler2D a;\nvoid fragment() {\n\tALBEDO = texture(screen_texture, SCREEN_UV).rgb * texture(a, UV).rgb + texture(n, UV).rgb;\n}\n";
        let (_, textures, _) = compile(source).unwrap();
        assert_eq!(textures, [TextureSource::Screen, TextureSource::Normal, TextureSource::White]);
        let error = compile("shader_type spatial;\nuniform sampler2D a;\nvoid fragment() {\n\tALBEDO = texture(a, 1.0);\n}\n").map(|_| ()).unwrap_err();
        assert!(error.starts_with("Line 4: "), "{error}");
    }

    #[test]
    fn half_res_sky_passes_hand_their_color_to_the_full_pass() {
        let source = "shader_type sky;\nrender_mode use_half_res_pass;\nvoid sky() {\n\tif (AT_HALF_RES_PASS) { COLOR = vec3(1.0); ALPHA = 0.5; } else { COLOR = HALF_RES_COLOR.rgb * HALF_RES_COLOR.a; }\n}\n";
        let t = translate(source).unwrap();
        assert!(t.glsl.contains("HALF_RES_COLOR = vec4(COLOR, ALPHA);"));
        assert!(compile(source).is_ok());
    }

    #[test]
    fn unsupported_and_hostile_sources_are_errors() {
        assert!(compile("void fragment() {}").is_err());
        assert!(compile("shader_type canvas_item;").is_err());
        assert!(compile("shader_type spatial;\nuniform samplerCube t;").unwrap_err().contains("sampler2D"));
        assert!(compile("shader_type spatial; }}}} /* unterminated").is_err());
        assert!(compile(&"x".repeat(MAX_SOURCE + 1)).is_err());
        assert!(compile("shader_type sky;\nuniform =;").is_err());
    }
}
