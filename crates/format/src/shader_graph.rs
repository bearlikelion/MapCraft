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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_position: Option<[f32; 2]>,
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
            positions: Vec::new(),
            output_position: None,
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
    /// Removes an unused node and keeps the remaining node references stable.
    pub fn remove_node(&mut self, id: usize) -> Result<(), GraphError> {
        if self.nodes.get(id).is_none() {
            return Err(error("node does not exist"));
        }
        if [self.albedo, self.roughness, self.metallic].contains(&id)
            || self.emission == Some(id)
            || self.nodes.iter().any(|node| match node {
                Node::Multiply { a, b } => *a == id || *b == id,
                Node::Sine { input } => *input == id,
                _ => false,
            })
        {
            return Err(error("node is connected; reconnect its consumers before deleting"));
        }
        let mut next = self.clone();
        next.nodes.remove(id);
        if next.positions.get(id).is_some() {
            next.positions.remove(id);
        }
        let renumber = |value: &mut usize| {
            if *value > id {
                *value = value.saturating_sub(1);
            }
        };
        for node in &mut next.nodes {
            match node {
                Node::Multiply { a, b } => {
                    renumber(a);
                    renumber(b);
                }
                Node::Sine { input } => renumber(input),
                _ => {}
            }
        }
        renumber(&mut next.albedo);
        renumber(&mut next.roughness);
        renumber(&mut next.metallic);
        if let Some(emission) = &mut next.emission {
            renumber(emission);
        }
        next.source()?;
        *self = next;
        Ok(())
    }

    /// Emits nodes in a deterministic dependency order, refusing missing inputs and cycles.
    pub fn source(&self) -> Result<String, GraphError> {
        if self.version != 1 {
            return Err(error(format!("unsupported version {}", self.version)));
        }
        if self.nodes.len() > MAX_NODES {
            return Err(error(format!("at most {MAX_NODES} nodes are supported")));
        }
        if self.positions.len() > MAX_NODES
            || !self.positions.iter().chain(self.output_position.iter()).flatten().all(|v| v.is_finite() && (0.0..=16384.0).contains(v))
        {
            return Err(error("node positions must be finite and within the canvas limits"));
        }
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut states = vec![0_u8; self.nodes.len()];
        for root in 0..self.nodes.len() {
            let mut pending = vec![(root, false)];
            while let Some((id, expanded)) = pending.pop() {
                let state = states.get_mut(id).ok_or_else(|| error(format!("input node {id} does not exist")))?;
                if *state == 2 {
                    continue;
                }
                if expanded {
                    *state = 2;
                    order.push(id);
                    continue;
                }
                if *state == 1 {
                    return Err(error(format!("node {id}: connection creates a cycle")));
                }
                *state = 1;
                pending.push((id, true));
                match self.nodes.get(id) {
                    Some(Node::Multiply { a, b }) => {
                        pending.push((*b, false));
                        pending.push((*a, false));
                    }
                    Some(Node::Sine { input }) => pending.push((*input, false)),
                    Some(_) => {}
                    None => return Err(error(format!("input node {id} does not exist"))),
                }
            }
        }
        let mut types = vec![None; self.nodes.len()];
        let mut code = String::from("shader_type spatial;\n\nvoid fragment() {\n");
        for id in order {
            let node = self.nodes.get(id).ok_or_else(|| error("node is missing"))?;
            let input_type = |input: usize| types.get(input).copied().flatten().ok_or_else(|| error(format!("node {id}: input {input} is missing")));
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
            if let Some(slot) = types.get_mut(id) {
                *slot = Some(ty);
            }
            let name = if ty == Type::Scalar { "float" } else { "vec3" };
            code.push_str(&format!("\t{name} n{id} = {expression};\n"));
        }
        for (name, id, expected) in
            [("ALBEDO", self.albedo, Type::Color), ("ROUGHNESS", self.roughness, Type::Scalar), ("METALLIC", self.metallic, Type::Scalar)]
        {
            if types.get(id).copied().flatten() != Some(expected) {
                return Err(error(format!("{name}: node {id} has a missing or incompatible output")));
            }
            code.push_str(&format!("\t{name} = n{id};\n"));
        }
        if let Some(id) = self.emission {
            if types.get(id).copied().flatten() != Some(Type::Color) {
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
    fn deletion_protects_connections_and_renumbers_survivors() {
        let mut graph = Graph::default();
        let original = graph.clone();
        assert!(graph.remove_node(0).is_err());
        assert_eq!(graph, original);
        graph.nodes.extend([Node::Time, Node::Scalar { value: 0.8 }]);
        graph.roughness = 4;
        graph.remove_node(3).unwrap();
        assert_eq!(graph.roughness, 3);
        assert_eq!(graph.nodes.get(3), Some(&Node::Scalar { value: 0.8 }));
        assert!(graph.remove_node(usize::MAX).is_err());
        assert!(graph.source().is_ok());
    }

    #[test]
    fn arbitrary_node_order_supports_dags_but_rejects_cycles() {
        let mut graph = Graph::default();
        graph.nodes.push(Node::Time);
        graph.nodes.insert(0, Node::Sine { input: 4 });
        graph.albedo = 1;
        graph.roughness = 2;
        graph.metallic = 3;
        let source = graph.source().unwrap();
        assert!(source.find("float n4 = TIME").unwrap() < source.find("float n0 = sin(n4)").unwrap());
        graph.nodes[4] = Node::Sine { input: 0 };
        assert!(graph.source().is_err());
    }

    #[test]
    fn layout_round_trips_without_changing_source() {
        let mut graph = Graph::default();
        let original = graph.source().unwrap();
        graph.positions = vec![[30.0, 80.0]];
        graph.output_position = Some([730.0, 150.0]);
        assert_eq!(graph.source().unwrap(), original);
        assert_eq!(serde_json::from_str::<Graph>(&serde_json::to_string(&graph).unwrap()).unwrap(), graph);
        graph.positions = vec![[f32::NAN, 0.0]];
        assert!(graph.source().is_err());
        graph.positions = vec![[0.0, 0.0]; MAX_NODES + 1];
        assert!(graph.source().is_err());
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
