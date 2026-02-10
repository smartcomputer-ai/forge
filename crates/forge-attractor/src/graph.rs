use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DurationValue {
    pub raw: String,
    pub millis: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AttrValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Duration(DurationValue),
}

impl AttrValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn to_string_value(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Boolean(value) => value.to_string(),
            Self::Duration(value) => value.raw.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Attributes {
    values: BTreeMap<String, AttrValue>,
    explicit_keys: BTreeSet<String>,
}

impl Attributes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn values(&self) -> &BTreeMap<String, AttrValue> {
        &self.values
    }

    pub fn set_inherited(&mut self, key: impl Into<String>, value: AttrValue) {
        self.values.insert(key.into(), value);
    }

    pub fn set_explicit(&mut self, key: impl Into<String>, value: AttrValue) {
        let key = key.into();
        self.explicit_keys.insert(key.clone());
        self.values.insert(key, value);
    }

    pub fn merge_inherited(&mut self, other: &Attributes) {
        for (key, value) in &other.values {
            self.values.insert(key.clone(), value.clone());
        }
    }

    pub fn merge_with_explicit_tracking(&mut self, other: &Attributes) {
        for (key, value) in &other.values {
            if other.explicit_keys.contains(key) {
                self.explicit_keys.insert(key.clone());
            }
            self.values.insert(key.clone(), value.clone());
        }
    }

    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.values.get(key)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(AttrValue::as_str)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(AttrValue::as_bool)
    }

    pub fn is_explicit(&self, key: &str) -> bool {
        self.explicit_keys.contains(key)
    }

    pub fn without_explicit(&self) -> Self {
        Self {
            values: self.values.clone(),
            explicit_keys: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub attrs: Attributes,
}

impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            attrs: Attributes::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub attrs: Attributes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub id: String,
    pub attrs: Attributes,
    pub nodes: BTreeMap<String, Node>,
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub source_dot: Option<String>,
}

impl Graph {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            attrs: Attributes::new(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            source_dot: None,
        }
    }

    pub fn outgoing_edges<'a>(&'a self, node_id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |edge| edge.from == node_id)
    }

    pub fn incoming_edges<'a>(&'a self, node_id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |edge| edge.to == node_id)
    }

    pub fn start_candidates(&self) -> Vec<&Node> {
        self.nodes
            .values()
            .filter(|node| {
                node.attrs.get_str("shape") == Some("Mdiamond")
                    || node.id == "start"
                    || node.id == "Start"
            })
            .collect()
    }

    pub fn terminal_candidates(&self) -> Vec<&Node> {
        self.nodes
            .values()
            .filter(|node| {
                node.attrs.get_str("shape") == Some("Msquare")
                    || matches!(node.id.to_ascii_lowercase().as_str(), "exit" | "end")
            })
            .collect()
    }
}
