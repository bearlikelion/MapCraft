use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("unknown variable ${{{0}}}")]
    UnknownVar(String),
    #[error("unclosed ${{ in `{0}`")]
    Unclosed(String),
    #[error("cannot start `{program}`: {source}")]
    Spawn { program: String, source: std::io::Error },
    #[error("`{program}` exited with {status}")]
    Failed { program: String, status: String },
    #[error("copy {from} to {to}: {source}")]
    Copy { from: String, to: String, source: std::io::Error },
    #[error("cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileProfile {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Step {
    Run {
        run: String,
        #[serde(default)]
        args: Vec<String>,
        /// Run a Windows tool through Wine on other platforms.
        #[serde(default)]
        wine: bool,
        #[serde(default)]
        cwd: Option<String>,
    },
    Copy {
        copy: String,
        to: String,
    },
    /// Writes the map with a plugin's exporter; the editor does this before the other steps run.
    Export {
        export: String,
        to: String,
    },
}

/// Replaces every `${name}` with its value; an unknown name is an error rather than an empty string.
pub fn expand(s: &str, vars: &BTreeMap<String, String>) -> Result<String, CompileError> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(rest.get(..start).unwrap_or_default());
        let after = rest.get(start + 2..).unwrap_or_default();
        let end = after.find('}').ok_or_else(|| CompileError::Unclosed(s.to_string()))?;
        let name = after.get(..end).unwrap_or_default();
        out.push_str(vars.get(name).ok_or_else(|| CompileError::UnknownVar(name.to_string()))?);
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

/// `map`, `map_dir`, `map_name`, `sdk_dir` and `cpu_count`.
pub fn builtin_vars(map: &Path, sdk_dir: &Path) -> BTreeMap<String, String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    BTreeMap::from([
        ("map".into(), s(map)),
        ("map_dir".into(), map.parent().map(s).unwrap_or_default()),
        ("map_name".into(), map.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        ("sdk_dir".into(), s(sdk_dir)),
        ("cpu_count".into(), std::thread::available_parallelism().map_or(1, |n| n.get()).to_string()),
    ])
}

fn pipe_lines(r: impl Read + Send + 'static, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let mut r = BufReader::new(r);
        let mut buf = Vec::new();
        while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
            let line = strip_ansi(String::from_utf8_lossy(&buf).trim_end_matches(['\n', '\r']));
            buf.clear();
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

/// `line` without the terminal escape sequences (colors, cursor moves) tools print even into pipes.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
        } else if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Runs the steps in order, streaming tool output line by line; stops at the first failure or when
/// `cancel` is set.
pub fn run_profile(
    profile: &CompileProfile,
    vars: &BTreeMap<String, String>,
    on_output: &mut dyn FnMut(&str),
    cancel: &AtomicBool,
) -> Result<(), CompileError> {
    for step in &profile.steps {
        if cancel.load(Ordering::Relaxed) {
            return Err(CompileError::Cancelled);
        }
        match step {
            Step::Export { export, to } => on_output(&format!("> export {export} {}", expand(to, vars)?)),
            Step::Copy { copy, to } => {
                let (from, to) = (expand(copy, vars)?, expand(to, vars)?);
                on_output(&format!("> copy {from} {to}"));
                let err = |source| CompileError::Copy { from: from.clone(), to: to.clone(), source };
                if let Some(parent) = Path::new(&to).parent() {
                    std::fs::create_dir_all(parent).map_err(err)?;
                }
                std::fs::copy(&from, &to).map_err(err)?;
            }
            Step::Run { run, args, wine, cwd } => {
                let mut program = expand(run, vars)?;
                let mut args = args.iter().map(|a| expand(a, vars)).collect::<Result<Vec<_>, _>>()?;
                if *wine && !cfg!(windows) {
                    args.insert(0, program);
                    program = "wine".into();
                }
                on_output(&format!("> {program} {}", args.join(" ")));
                let mut cmd = Command::new(&program);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
                if let Some(dir) = cwd {
                    cmd.current_dir(expand(dir, vars)?);
                }
                let mut child = cmd.spawn().map_err(|source| CompileError::Spawn { program: program.clone(), source })?;
                let (tx, rx) = mpsc::channel();
                if let Some(out) = child.stdout.take() {
                    pipe_lines(out, tx.clone());
                }
                if let Some(err) = child.stderr.take() {
                    pipe_lines(err, tx);
                }
                loop {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(line) => on_output(&line),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if cancel.load(Ordering::Relaxed) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(CompileError::Cancelled);
                    }
                }
                let status = child.wait().map_err(|source| CompileError::Spawn { program: program.clone(), source })?;
                if !status.success() {
                    return Err(CompileError::Failed { program, status: status.to_string() });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_terminal_colors() {
        let line = "[  16% ] \u{1b}[90m\u{1b}[1mloading_editor_layout\u{1b}[22m | Loading docks...\u{1b}[39m\u{1b}[0m";
        assert_eq!(strip_ansi(line), "[  16% ] loading_editor_layout | Loading docks...");
    }

    #[test]
    fn export_steps_parse_and_log() {
        let profile: CompileProfile = toml::from_str(
            r#"
name = "zones"
steps = [{ export = "vmf", to = "${dir}/x.vmf" }]
"#,
        )
        .unwrap();
        assert_eq!(profile.steps, vec![Step::Export { export: "vmf".into(), to: "${dir}/x.vmf".into() }]);
        let vars = BTreeMap::from([("dir".to_string(), "/tmp".to_string())]);
        let mut lines = Vec::new();
        run_profile(&profile, &vars, &mut |l| lines.push(l.to_string()), &AtomicBool::new(false)).unwrap();
        assert_eq!(lines, vec!["> export vmf /tmp/x.vmf".to_string()], "the editor writes exports; the runner only reports them");
    }

    #[test]
    fn expansion() {
        let vars = BTreeMap::from([("a".to_string(), "1".to_string()), ("bb".to_string(), "two".to_string())]);
        assert_eq!(expand("x${a}y${bb}${a}", &vars).unwrap(), "x1ytwo1");
        assert_eq!(expand("plain $ text", &vars).unwrap(), "plain $ text");
        assert!(matches!(expand("${missing}", &vars), Err(CompileError::UnknownVar(v)) if v == "missing"));
        assert!(matches!(expand("${a", &vars), Err(CompileError::Unclosed(_))));
        assert_eq!(builtin_vars(Path::new("/maps/surf_x.mcmap"), Path::new("/sdk"))["map_name"], "surf_x");
    }

    #[cfg(unix)]
    #[test]
    fn runs_steps() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, "hi").unwrap();
        let vars = BTreeMap::from([("dir".to_string(), dir.path().to_string_lossy().into_owned())]);
        let profile: CompileProfile = toml::from_str(
            r#"
name = "test"
steps = [
    { copy = "${dir}/a.txt", to = "${dir}/out/b.txt" },
    { run = "sh", args = ["-c", "echo built; echo warn >&2"] },
]
"#,
        )
        .unwrap();
        let mut lines = Vec::new();
        run_profile(&profile, &vars, &mut |l| lines.push(l.to_string()), &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("out/b.txt")).unwrap(), "hi");
        assert!(lines.contains(&"built".to_string()) && lines.contains(&"warn".to_string()));

        let failing =
            CompileProfile { name: "f".into(), steps: vec![Step::Run { run: "sh".into(), args: vec!["-c".into(), "exit 3".into()], wine: false, cwd: None }] };
        assert!(matches!(run_profile(&failing, &vars, &mut |_| {}, &AtomicBool::new(false)), Err(CompileError::Failed { .. })));
        let missing = CompileProfile { name: "m".into(), steps: vec![Step::Run { run: "/no/such/tool".into(), args: vec![], wine: false, cwd: None }] };
        assert!(matches!(run_profile(&missing, &vars, &mut |_| {}, &AtomicBool::new(false)), Err(CompileError::Spawn { .. })));
        assert!(matches!(run_profile(&profile, &vars, &mut |_| {}, &AtomicBool::new(true)), Err(CompileError::Cancelled)));
    }
}
