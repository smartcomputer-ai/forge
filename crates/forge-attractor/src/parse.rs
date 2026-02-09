use crate::{
    AttrValue, Attributes, AttractorError, DurationValue, Edge, Graph, Node,
};
use graphviz_rust::dot_structures::{
    Attribute, Edge as DotEdge, EdgeTy, Graph as DotGraph, GraphAttributes, Id, Node as DotNode,
    NodeId, Stmt, Subgraph, Vertex,
};
#[derive(Clone, Debug, Default)]
struct Scope {
    node_defaults: Attributes,
    edge_defaults: Attributes,
    classes: Vec<String>,
}

#[derive(Debug)]
struct ParseState {
    graph: Graph,
}

impl ParseState {
    fn new(id: String) -> Self {
        Self {
            graph: Graph::new(id),
        }
    }
}

pub fn parse_dot(source: &str) -> Result<Graph, AttractorError> {
    if has_undirected_edge_token(source) {
        return Err(AttractorError::InvalidGraph(
            "undirected edge token '--' is not supported".to_string(),
        ));
    }

    let normalized = normalize_duration_literals(source);
    let dot_graph = graphviz_rust::parse(&normalized).map_err(AttractorError::DotParse)?;
    convert_graph(dot_graph)
}

fn convert_graph(graph: DotGraph) -> Result<Graph, AttractorError> {
    let (graph_id, strict, is_digraph, stmts) = match graph {
        DotGraph::DiGraph { id, strict, stmts } => (dot_id_to_string(id)?, strict, true, stmts),
        DotGraph::Graph { id, strict, stmts } => (dot_id_to_string(id)?, strict, false, stmts),
    };

    if !is_digraph {
        return Err(AttractorError::InvalidGraph(
            "only 'digraph' is supported".to_string(),
        ));
    }
    if strict {
        return Err(AttractorError::InvalidGraph(
            "'strict' graphs are not supported".to_string(),
        ));
    }

    let mut state = ParseState::new(graph_id);
    let scope = Scope::default();
    process_statements(&mut state, &stmts, &scope, true)?;
    Ok(state.graph)
}

fn process_statements(
    state: &mut ParseState,
    stmts: &[Stmt],
    parent_scope: &Scope,
    top_level: bool,
) -> Result<(), AttractorError> {
    let mut scope = parent_scope.clone();

    for stmt in stmts {
        match stmt {
            Stmt::GAttribute(graph_attrs) => match graph_attrs {
                GraphAttributes::Node(attrs) => {
                    let parsed = parse_attributes(attrs)?;
                    scope.node_defaults.merge_inherited(&parsed);
                }
                GraphAttributes::Edge(attrs) => {
                    let parsed = parse_attributes(attrs)?;
                    scope.edge_defaults.merge_inherited(&parsed);
                }
                GraphAttributes::Graph(attrs) => {
                    if top_level {
                        let parsed = parse_attributes(attrs)?;
                        state.graph.attrs.merge_inherited(&parsed);
                    }
                }
            },
            Stmt::Attribute(attr) => {
                if top_level {
                    let (key, value) = parse_attribute(attr)?;
                    state.graph.attrs.set_explicit(key, value);
                }
            }
            Stmt::Node(node) => process_node_stmt(state, node, &scope)?,
            Stmt::Edge(edge) => process_edge_stmt(state, edge, &scope)?,
            Stmt::Subgraph(subgraph) => process_subgraph_stmt(state, subgraph, &scope)?,
        }
    }

    Ok(())
}

fn process_subgraph_stmt(
    state: &mut ParseState,
    subgraph: &Subgraph,
    parent_scope: &Scope,
) -> Result<(), AttractorError> {
    let mut scope = parent_scope.clone();
    if let Some(class_name) = derive_subgraph_class(subgraph)? {
        scope.classes.push(class_name);
    }

    process_statements(state, &subgraph.stmts, &scope, false)
}

fn derive_subgraph_class(subgraph: &Subgraph) -> Result<Option<String>, AttractorError> {
    let mut label: Option<String> = None;

    for stmt in &subgraph.stmts {
        match stmt {
            Stmt::Attribute(Attribute(key, value)) => {
                if id_to_attr_key(key)? == "label" {
                    label = Some(id_to_string(value)?);
                }
            }
            Stmt::GAttribute(GraphAttributes::Graph(attrs)) => {
                for attr in attrs {
                    let (key, value) = parse_attribute(attr)?;
                    if key == "label" {
                        label = Some(value.to_string_value());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(label.and_then(|label| {
        let mut out = String::new();
        let mut prev_dash = false;
        for ch in label.trim().to_ascii_lowercase().chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
                prev_dash = false;
            } else if ch.is_ascii_whitespace() || ch == '-' {
                if !prev_dash && !out.is_empty() {
                    out.push('-');
                    prev_dash = true;
                }
            }
        }
        if out.ends_with('-') {
            out.pop();
        }
        if out.is_empty() { None } else { Some(out) }
    }))
}

fn process_node_stmt(state: &mut ParseState, node: &DotNode, scope: &Scope) -> Result<(), AttractorError> {
    let node_id = parse_node_id(&node.id)?;

    let mut attrs = scope.node_defaults.without_explicit();
    let parsed = parse_attributes(&node.attributes)?;
    attrs.merge_with_explicit_tracking(&parsed);

    if !scope.classes.is_empty() {
        let mut classes = parse_class_list(attrs.get_str("class").unwrap_or_default());
        for class_name in &scope.classes {
            if !classes.iter().any(|existing| existing == class_name) {
                classes.push(class_name.clone());
            }
        }
        if !classes.is_empty() {
            attrs.set_inherited("class", AttrValue::String(classes.join(",")));
        }
    }

    let entry = state
        .graph
        .nodes
        .entry(node_id.clone())
        .or_insert_with(|| Node::new(node_id));
    entry.attrs.merge_with_explicit_tracking(&attrs);
    Ok(())
}

fn process_edge_stmt(state: &mut ParseState, edge: &DotEdge, scope: &Scope) -> Result<(), AttractorError> {
    let vertices = match &edge.ty {
        EdgeTy::Pair(from, to) => vec![parse_vertex(from)?, parse_vertex(to)?],
        EdgeTy::Chain(chain) => {
            let mut result = Vec::with_capacity(chain.len());
            for vertex in chain {
                result.push(parse_vertex(vertex)?);
            }
            result
        }
    };

    if vertices.len() < 2 {
        return Err(AttractorError::InvalidGraph(
            "edge chain must contain at least two vertices".to_string(),
        ));
    }

    let mut attrs = scope.edge_defaults.without_explicit();
    let parsed = parse_attributes(&edge.attributes)?;
    attrs.merge_with_explicit_tracking(&parsed);

    for pair in vertices.windows(2) {
        let from = pair[0].clone();
        let to = pair[1].clone();
        state.graph.edges.push(Edge {
            from,
            to,
            attrs: attrs.clone(),
        });
    }

    Ok(())
}

fn parse_vertex(vertex: &Vertex) -> Result<String, AttractorError> {
    match vertex {
        Vertex::N(node_id) => parse_node_id(node_id),
        Vertex::S(_) => Err(AttractorError::InvalidGraph(
            "subgraph vertices in edge statements are not supported in Attractor subset"
                .to_string(),
        )),
    }
}

fn parse_node_id(node_id: &NodeId) -> Result<String, AttractorError> {
    if node_id.1.is_some() {
        return Err(AttractorError::InvalidGraph(
            "ports in node identifiers are not supported".to_string(),
        ));
    }

    let id = id_to_identifier(&node_id.0)?;
    Ok(id)
}

fn parse_attributes(attrs: &[Attribute]) -> Result<Attributes, AttractorError> {
    let mut parsed = Attributes::new();
    for attr in attrs {
        let (key, value) = parse_attribute(attr)?;
        parsed.set_explicit(key, value);
    }
    Ok(parsed)
}

fn parse_attribute(attr: &Attribute) -> Result<(String, AttrValue), AttractorError> {
    let key = id_to_attr_key(&attr.0)?;
    let value = parse_attr_value(&attr.1)?;
    Ok((key, value))
}

fn dot_id_to_string(id: Id) -> Result<String, AttractorError> {
    match id {
        Id::Anonymous(value) => Ok(value),
        other => id_to_identifier(&other),
    }
}

fn id_to_attr_key(id: &Id) -> Result<String, AttractorError> {
    let key = id_to_string(id)?;
    if is_valid_attr_key(&key) {
        Ok(key)
    } else {
        Err(AttractorError::InvalidGraph(format!(
            "invalid attribute key '{key}'"
        )))
    }
}

fn parse_attr_value(id: &Id) -> Result<AttrValue, AttractorError> {
    match id {
        Id::Html(_) => Err(AttractorError::InvalidGraph(
            "HTML attribute values are not supported".to_string(),
        )),
        Id::Escaped(_) => {
            let value = id_to_string(id)?;
            if let Some(duration) = parse_duration(&value) {
                Ok(AttrValue::Duration(duration))
            } else {
                Ok(AttrValue::String(value))
            }
        }
        Id::Plain(raw) => {
            if raw == "true" {
                return Ok(AttrValue::Boolean(true));
            }
            if raw == "false" {
                return Ok(AttrValue::Boolean(false));
            }
            if let Some(duration) = parse_duration(raw) {
                return Ok(AttrValue::Duration(duration));
            }
            if let Ok(value) = raw.parse::<i64>() {
                return Ok(AttrValue::Integer(value));
            }
            if raw.contains('.') {
                if let Ok(value) = raw.parse::<f64>() {
                    return Ok(AttrValue::Float(value));
                }
            }
            Ok(AttrValue::String(raw.clone()))
        }
        Id::Anonymous(value) => Ok(AttrValue::String(value.clone())),
    }
}

fn parse_duration(raw: &str) -> Option<DurationValue> {
    if raw.len() < 2 {
        return None;
    }

    let units = ["ms", "s", "m", "h", "d"];
    let unit = units.iter().find(|unit| raw.ends_with(**unit))?;
    let number_part = &raw[..raw.len() - unit.len()];
    let value = number_part.parse::<u64>().ok()?;

    let factor = match *unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };

    Some(DurationValue {
        raw: raw.to_string(),
        millis: value.saturating_mul(factor),
    })
}

fn id_to_identifier(id: &Id) -> Result<String, AttractorError> {
    let value = id_to_string(id)?;
    if is_valid_identifier(&value) {
        Ok(value)
    } else {
        Err(AttractorError::InvalidGraph(format!(
            "node id '{value}' is invalid; expected [A-Za-z_][A-Za-z0-9_]*"
        )))
    }
}

fn id_to_string(id: &Id) -> Result<String, AttractorError> {
    match id {
        Id::Plain(value) => Ok(value.clone()),
        Id::Escaped(value) => {
            let unquoted = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .ok_or_else(|| {
                    AttractorError::InvalidGraph(format!(
                        "escaped string id '{value}' is missing quotes"
                    ))
                })?;
            Ok(unescape_dot_string(unquoted))
        }
        Id::Html(_) => Err(AttractorError::InvalidGraph(
            "HTML labels/IDs are not supported".to_string(),
        )),
        Id::Anonymous(value) => Ok(value.clone()),
    }
}

fn unescape_dot_string(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('"') => output.push('"'),
                Some('\\') => output.push('\\'),
                Some(other) => output.push(other),
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }

    output
}

fn parse_class_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_valid_attr_key(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let parts: Vec<&str> = value.split('.').collect();
    if parts.is_empty() {
        return false;
    }
    for part in parts {
        if part.is_empty() {
            return false;
        }
        let mut chars = part.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
            _ => return false,
        }
        if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            return false;
        }
    }
    true
}

fn has_undirected_edge_token(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while index < bytes.len() {
        let current = bytes[index] as char;
        let next = if index + 1 < bytes.len() {
            Some(bytes[index + 1] as char)
        } else {
            None
        };

        if in_line_comment {
            if current == '\n' {
                in_line_comment = false;
            }
            index += 1;
            continue;
        }

        if in_block_comment {
            if current == '*' && next == Some('/') {
                in_block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if in_string {
            if current == '\\' {
                index += 2;
                continue;
            }
            if current == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if current == '/' && next == Some('/') {
            in_line_comment = true;
            index += 2;
            continue;
        }
        if current == '/' && next == Some('*') {
            in_block_comment = true;
            index += 2;
            continue;
        }
        if current == '"' {
            in_string = true;
            index += 1;
            continue;
        }

        if current == '-' && next == Some('-') {
            return true;
        }

        index += 1;
    }

    false
}

fn normalize_duration_literals(source: &str) -> String {
    let mut output = String::with_capacity(source.len() + 16);
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while index < bytes.len() {
        let current = bytes[index] as char;
        let next = if index + 1 < bytes.len() {
            Some(bytes[index + 1] as char)
        } else {
            None
        };

        if in_line_comment {
            output.push(current);
            if current == '\n' {
                in_line_comment = false;
            }
            index += 1;
            continue;
        }

        if in_block_comment {
            output.push(current);
            if current == '*' && next == Some('/') {
                output.push('/');
                in_block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if in_string {
            output.push(current);
            if current == '\\' && next.is_some() {
                output.push(next.expect("next char exists"));
                index += 2;
                continue;
            }
            if current == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if current == '/' && next == Some('/') {
            output.push(current);
            output.push('/');
            in_line_comment = true;
            index += 2;
            continue;
        }
        if current == '/' && next == Some('*') {
            output.push(current);
            output.push('*');
            in_block_comment = true;
            index += 2;
            continue;
        }
        if current == '"' {
            output.push(current);
            in_string = true;
            index += 1;
            continue;
        }

        if current == '=' {
            output.push(current);
            index += 1;

            while index < bytes.len() {
                let ch = bytes[index] as char;
                if ch.is_whitespace() {
                    output.push(ch);
                    index += 1;
                } else {
                    break;
                }
            }

            if index >= bytes.len() || bytes[index] as char == '"' {
                continue;
            }

            let token_start = index;
            while index < bytes.len() {
                let ch = bytes[index] as char;
                if ch.is_ascii_alphanumeric() {
                    index += 1;
                } else {
                    break;
                }
            }

            if token_start == index {
                continue;
            }

            let token = &source[token_start..index];
            if parse_duration(token).is_some() {
                output.push('"');
                output.push_str(token);
                output.push('"');
            } else {
                output.push_str(token);
            }
            continue;
        }

        output.push(current);
        index += 1;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dot_linear_graph_expected_nodes_and_edges() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan [prompt="Do thing"]
                exit [shape=Msquare]
                start -> plan -> exit
            }
            "#,
        )
        .expect("graph should parse");

        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert!(graph.nodes.contains_key("plan"));
    }

    #[test]
    fn parse_dot_subgraph_derives_class_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                subgraph cluster_loop {
                    label="Loop A"
                    node [timeout=900s]
                    Plan
                }
            }
            "#,
        )
        .expect("graph should parse");

        let node = graph.nodes.get("Plan").expect("node should exist");
        assert_eq!(node.attrs.get_str("class"), Some("loop-a"));
        assert!(matches!(
            node.attrs.get("timeout"),
            Some(AttrValue::Duration(DurationValue { millis: 900_000, .. }))
        ));
    }

    #[test]
    fn parse_dot_undirected_edge_rejected_expected_error() {
        let error = parse_dot("digraph G { a -- b }").expect_err("must fail");
        assert!(error.to_string().contains("undirected edge token"));
    }

    #[test]
    fn parse_dot_html_label_rejected_expected_error() {
        let error = parse_dot("digraph G { a [label=<<b>>] }").expect_err("must fail");
        assert!(error.to_string().contains("HTML"));
    }

    #[test]
    fn parse_duration_value_valid_expected_millis() {
        let duration = parse_duration("2h").expect("duration must parse");
        assert_eq!(duration.millis, 7_200_000);
    }

    #[test]
    fn normalize_duration_literals_unquoted_expected_quoted() {
        let normalized = normalize_duration_literals("digraph G { a [timeout=900s] }");
        assert!(normalized.contains("timeout=\"900s\""));
    }
}
