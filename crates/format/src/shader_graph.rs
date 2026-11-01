//! Bounded, typed shader graphs that generate Godot spatial shader source.

use serde::{Deserialize, Serialize};

pub const MAX_NODES: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Scalar { value: f32 },
    Color { value: [f32; 3] },
    Time,
    Multiply { a: usize, b: usize },
    Sine { input: usize },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub version: u32,
    pub nodes: Vec<Node>,
    pub albedo: usize,
    pub roughness: usize,
    pub metallic: usize,
    pub emission: Option<usize>,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            version: 1,
            nodes: vec![Node::Color { value: [0.6, 0.7, 0.8] }, Node::Scalar { value: 0.5 }, Node::Scalar { value: 0.0 }],
            albedo: 0,
            roughness: 1,
            metallic: 2,
            emission: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    Scalar,
    Color,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid shader graph: {0}")]
pub struct GraphError(String);

fn error(message: impl Into<String>) -> GraphError {
    GraphError(message.into())
}

impl Graph {
    /// Inputs must reference earlier nodes, so cycles and forward references are rejected.
    pub fn source(&self) -> Result<String, GraphError> {
        if self.version != 1 {
            return Err(error(format!("unsupported version {}", self.version)));
        }
        if self.nodes.len() > MAX_NODES {
            return Err(error(format!("at most {MAX_NODES} nodes are supported")));
        }
        let mut types = Vec::with_capacity(self.nodes.len());
        let mut code = String::from("shader_type spatial;\n\nvoid fragment() {\n");
        for (id, node) in self.nodes.iter().enumerate() {
            let input_type = |input: usize| types.get(input).copied().ok_or_else(|| error(format!("node {id}: input {input} must reference an earlier node")));
            let (ty, expression) = match node {
                Node::Scalar { value } => {
                    if !value.is_finite() {
                        return Err(error(format!("node {id}: value must be finite")));
                    }
                    (Type::Scalar, format!("{value:?}"))
                }
                Node::Color { value: [r, g, b] } => {
                    if ![r, g, b].iter().all(|v| v.is_finite()) {
                        return Err(error(format!("node {id}: color must be finite")));
                    }
                    (Type::Color, format!("vec3({r:?}, {g:?}, {b:?})"))
                }
                Node::Time => (Type::Scalar, "TIME".into()),
                Node::Multiply { a, b } => {
                    let a_type = input_type(*a)?;
                    let b_type = input_type(*b)?;
                    let ty = if a_type == Type::Color || b_type == Type::Color { Type::Color } else { Type::Scalar };
                    (ty, format!("n{a} * n{b}"))
                }
                Node::Sine { input } => (input_type(*input)?, format!("sin(n{input})")),
            };
            types.push(ty);
            let name = if ty == Type::Scalar { "float" } else { "vec3" };
            code.push_str(&format!("\t{name} n{id} = {expression};\n"));
        }
        for (name, id, expected) in
            [("ALBEDO", self.albedo, Type::Color), ("ROUGHNESS", self.roughness, Type::Scalar), ("METALLIC", self.metallic, Type::Scalar)]
        {
            if types.get(id) != Some(&expected) {
                return Err(error(format!("{name}: node {id} has a missing or incompatible output")));
            }
            code.push_str(&format!("\t{name} = n{id};\n"));
        }
        if let Some(id) = self.emission {
            if types.get(id) != Some(&Type::Color) {
                return Err(error("EMISSION requires a color node"));
            }
            code.push_str(&format!("\tEMISSION = n{id};\n"));
        }
        code.push_str("}\n");
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_graph_generates_deterministic_spatial_source() {
        let graph = Graph::default();
        let source = graph.source().unwrap();
        assert!(source.contains("vec3 n0 = vec3(0.6, 0.7, 0.8);"));
        assert!(source.contains("ROUGHNESS = n1;"));
        assert_eq!(Graph::default().source().unwrap(), source);
        let encoded = serde_json::to_string(&graph).unwrap();
        assert_eq!(serde_json::from_str::<Graph>(&encoded).unwrap(), graph);
    }

    #[test]
    fn animation_and_scalar_color_multiplication() {
        let mut graph = Graph::default();
        graph.nodes.extend([Node::Time, Node::Sine { input: 3 }, Node::Multiply { a: 0, b: 4 }]);
        graph.emission = Some(5);
        let source = graph.source().unwrap();
        assert!(source.contains("float n4 = sin(n3);"));
        assert!(source.contains("vec3 n5 = n0 * n4;"));
        assert!(source.contains("EMISSION = n5;"));
    }

    #[test]
    fn hostile_graphs_return_errors() {
        let mut graph = Graph::default();
        graph.nodes.push(Node::Sine { input: usize::MAX });
        assert!(graph.source().is_err());
        graph.nodes.pop();
        graph.nodes.push(Node::Sine { input: 3 });
        assert!(graph.source().is_err());
        graph.nodes.pop();
        graph.albedo = 1;
        assert!(graph.source().is_err());
        graph.albedo = 0;
        graph.nodes.push(Node::Scalar { value: f32::NAN });
        assert!(graph.source().is_err());
        graph.nodes = vec![Node::Time; MAX_NODES + 1];
        assert!(graph.source().is_err());
        graph.version = 999;
        assert!(graph.source().is_err());
    }
}
