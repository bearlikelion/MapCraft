use mapcraft_geom::DVec3;
use serde::{Deserialize, Serialize};

/// Key/values in file order; `classname` is kept out of `properties`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub classname: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
}

/// An entity output wired to a target's input (Hammer I/O).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub output: String,
    pub target: String,
    pub input: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub param: String,
    #[serde(default)]
    pub delay: f64,
    /// -1 fires every time.
    #[serde(default = "always")]
    pub times_to_fire: i32,
}

fn always() -> i32 {
    -1
}

impl Entity {
    pub fn new(classname: impl Into<String>) -> Self {
        Self { classname: classname.into(), ..Self::default() }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        if key == "classname" {
            return Some(&self.classname);
        }
        self.properties.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let (key, value) = (key.into(), value.into());
        if key == "classname" {
            self.classname = value;
        } else if let Some(slot) = self.properties.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.properties.push((key, value));
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        let i = self.properties.iter().position(|(k, _)| k == key)?;
        Some(self.properties.remove(i).1)
    }

    pub fn origin(&self) -> Option<DVec3> {
        parse_vec3(self.get("origin")?)
    }
}

pub fn parse_vec3(s: &str) -> Option<DVec3> {
    let mut it = s.split_whitespace().map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite()));
    let v = DVec3::new(it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties() {
        let mut e = Entity::new("info_player_start");
        e.set("origin", "1 2 3");
        e.set("angles", "0 90 0");
        e.set("origin", "4 5 6");
        assert_eq!(e.properties.len(), 2);
        assert_eq!(e.origin(), Some(DVec3::new(4.0, 5.0, 6.0)));
        assert_eq!(e.get("classname"), Some("info_player_start"));
        assert_eq!(e.remove("angles").as_deref(), Some("0 90 0"));
        assert_eq!(parse_vec3("1 2"), None);
        assert_eq!(parse_vec3("1 2 3 4"), None);
        assert_eq!(parse_vec3("1 nan 3"), None);
    }
}
