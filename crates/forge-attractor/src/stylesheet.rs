use crate::{AttrValue, AttractorError, Graph};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Universal,
    NodeId(String),
    Class(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StylesheetRule {
    pub selector: Selector,
    pub declarations: Vec<(String, String)>,
    pub order: usize,
}

impl StylesheetRule {
    fn specificity(&self) -> usize {
        match self.selector {
            Selector::Universal => 0,
            Selector::Class(_) => 1,
            Selector::NodeId(_) => 2,
        }
    }

    fn matches_node(&self, node_id: &str, classes: &[String]) -> bool {
        match &self.selector {
            Selector::Universal => true,
            Selector::NodeId(id) => id == node_id,
            Selector::Class(class_name) => classes.iter().any(|class| class == class_name),
        }
    }
}

pub fn parse_stylesheet(input: &str) -> Result<Vec<StylesheetRule>, AttractorError> {
    let mut rules = Vec::new();
    let mut cursor = 0usize;
    let bytes = input.as_bytes();

    while cursor < bytes.len() {
        skip_whitespace(input, &mut cursor);
        if cursor >= bytes.len() {
            break;
        }

        let selector_start = cursor;
        while cursor < bytes.len() && bytes[cursor] as char != '{' {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(AttractorError::StylesheetParse(
                "missing '{' after selector".to_string(),
            ));
        }

        let selector_raw = input[selector_start..cursor].trim();
        let selector = parse_selector(selector_raw)?;
        cursor += 1;

        let block_start = cursor;
        while cursor < bytes.len() && bytes[cursor] as char != '}' {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(AttractorError::StylesheetParse(
                "missing closing '}' for stylesheet rule".to_string(),
            ));
        }

        let block = &input[block_start..cursor];
        cursor += 1;

        let declarations = parse_declarations(block)?;
        rules.push(StylesheetRule {
            selector,
            declarations,
            order: rules.len(),
        });
    }

    Ok(rules)
}

pub fn apply_model_stylesheet(graph: &mut Graph) -> Result<(), AttractorError> {
    let stylesheet = graph.attrs.get_str("model_stylesheet").unwrap_or_default();
    if stylesheet.trim().is_empty() {
        return Ok(());
    }

    let rules = parse_stylesheet(stylesheet)?;
    let recognized = ["llm_model", "llm_provider", "reasoning_effort"];

    for node in graph.nodes.values_mut() {
        let node_classes = parse_class_list(node.attrs.get_str("class").unwrap_or_default());

        for property in recognized {
            if node.attrs.is_explicit(property) {
                continue;
            }

            let mut selected: Option<(usize, usize, String)> = None;
            for rule in &rules {
                if !rule.matches_node(&node.id, &node_classes) {
                    continue;
                }

                if let Some((_, value)) = rule
                    .declarations
                    .iter()
                    .find(|(rule_property, _)| rule_property == property)
                {
                    let candidate = (rule.specificity(), rule.order, value.clone());
                    match &selected {
                        Some((specificity, order, _))
                            if *specificity > candidate.0
                                || (*specificity == candidate.0 && *order > candidate.1) => {}
                        _ => selected = Some(candidate),
                    }
                }
            }

            if let Some((_, _, value)) = selected {
                node.attrs
                    .set_inherited(property.to_string(), AttrValue::String(value));
            }
        }
    }

    Ok(())
}

fn parse_selector(selector_raw: &str) -> Result<Selector, AttractorError> {
    if selector_raw.is_empty() {
        return Err(AttractorError::StylesheetParse(
            "empty selector is invalid".to_string(),
        ));
    }

    if selector_raw == "*" {
        return Ok(Selector::Universal);
    }

    if let Some(rest) = selector_raw.strip_prefix('#') {
        if !is_identifier(rest) {
            return Err(AttractorError::StylesheetParse(format!(
                "invalid node id selector '#{rest}'"
            )));
        }
        return Ok(Selector::NodeId(rest.to_string()));
    }

    if let Some(rest) = selector_raw.strip_prefix('.') {
        if !is_class_name(rest) {
            return Err(AttractorError::StylesheetParse(format!(
                "invalid class selector '.{rest}'"
            )));
        }
        return Ok(Selector::Class(rest.to_string()));
    }

    Err(AttractorError::StylesheetParse(format!(
        "unsupported selector '{selector_raw}'"
    )))
}

fn parse_declarations(block: &str) -> Result<Vec<(String, String)>, AttractorError> {
    let mut declarations = Vec::new();

    for declaration in block.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }

        let (property, value) = declaration
            .split_once(':')
            .ok_or_else(|| {
                AttractorError::StylesheetParse(format!(
                    "declaration '{declaration}' is missing ':'"
                ))
            })?;

        let property = property.trim();
        let value = value.trim();
        if value.is_empty() {
            return Err(AttractorError::StylesheetParse(format!(
                "property '{property}' must have a non-empty value"
            )));
        }

        if !matches!(property, "llm_model" | "llm_provider" | "reasoning_effort") {
            return Err(AttractorError::StylesheetParse(format!(
                "property '{property}' is not supported"
            )));
        }

        let normalized = if value.starts_with('"') {
            value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .ok_or_else(|| {
                    AttractorError::StylesheetParse(format!(
                        "value '{value}' has unmatched quotes"
                    ))
                })?
                .to_string()
        } else {
            value.to_string()
        };

        if property == "reasoning_effort"
            && !matches!(normalized.as_str(), "low" | "medium" | "high")
        {
            return Err(AttractorError::StylesheetParse(format!(
                "reasoning_effort '{normalized}' must be low|medium|high"
            )));
        }

        declarations.push((property.to_string(), normalized));
    }

    if declarations.is_empty() {
        return Err(AttractorError::StylesheetParse(
            "stylesheet rule must contain at least one declaration".to_string(),
        ));
    }

    Ok(declarations)
}

fn parse_class_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn skip_whitespace(input: &str, cursor: &mut usize) {
    let bytes = input.as_bytes();
    while *cursor < bytes.len() && (bytes[*cursor] as char).is_whitespace() {
        *cursor += 1;
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_class_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_dot, AttrValue};

    #[test]
    fn parse_stylesheet_valid_rules_expected_count() {
        let rules = parse_stylesheet(
            r#"
            * { llm_model: "m1"; llm_provider: openai; }
            .code { llm_model: m2; }
            #critical { reasoning_effort: high; }
            "#,
        )
        .expect("stylesheet should parse");

        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn apply_model_stylesheet_specificity_expected_override() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="
                    * { llm_model: base; llm_provider: openai; }
                    .code { llm_model: class_model; }
                    #critical_review { llm_model: id_model; reasoning_effort: high; }
                "]
                critical_review [class="code"]
            }
            "#,
        )
        .expect("graph should parse");

        apply_model_stylesheet(&mut graph).expect("stylesheet should apply");
        let node = graph.nodes.get("critical_review").expect("node should exist");

        assert_eq!(
            node.attrs.get("llm_model"),
            Some(&AttrValue::String("id_model".to_string()))
        );
        assert_eq!(
            node.attrs.get("reasoning_effort"),
            Some(&AttrValue::String("high".to_string()))
        );
    }
}
