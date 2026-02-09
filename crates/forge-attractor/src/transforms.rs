use crate::{apply_model_stylesheet, lint::LintRule, validate, AttrValue, AttractorError, Diagnostic, Graph};

pub trait Transform: Send + Sync {
    fn apply(&self, graph: &mut Graph) -> Result<(), AttractorError>;
}

#[derive(Clone, Debug, Default)]
pub struct VariableExpansionTransform;

impl Transform for VariableExpansionTransform {
    fn apply(&self, graph: &mut Graph) -> Result<(), AttractorError> {
        let goal = graph.attrs.get_str("goal").unwrap_or_default().to_string();
        if goal.is_empty() {
            return Ok(());
        }

        for node in graph.nodes.values_mut() {
            if let Some(prompt) = node.attrs.get_str("prompt") {
                let replaced = prompt.replace("$goal", &goal);
                node.attrs.set_inherited("prompt", AttrValue::String(replaced));
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelStylesheetTransform;

impl Transform for ModelStylesheetTransform {
    fn apply(&self, graph: &mut Graph) -> Result<(), AttractorError> {
        apply_model_stylesheet(graph)
    }
}

pub fn apply_builtin_transforms(graph: &mut Graph) -> Result<(), AttractorError> {
    VariableExpansionTransform.apply(graph)?;
    ModelStylesheetTransform.apply(graph)?;
    Ok(())
}

pub fn prepare_pipeline(
    dot_source: &str,
    custom_transforms: &[&dyn Transform],
    extra_rules: &[&dyn LintRule],
) -> Result<(Graph, Vec<Diagnostic>), AttractorError> {
    let mut graph = crate::parse_dot(dot_source)?;
    apply_builtin_transforms(&mut graph)?;

    for transform in custom_transforms {
        transform.apply(&mut graph)?;
    }

    let diagnostics = validate(&graph, extra_rules);
    Ok((graph, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[test]
    fn variable_expansion_transform_goal_expected_prompt_expanded() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [goal="Ship feature"]
                plan [prompt="Plan for $goal"]
            }
            "#,
        )
        .expect("graph should parse");

        VariableExpansionTransform
            .apply(&mut graph)
            .expect("transform should apply");

        let plan = graph.nodes.get("plan").expect("plan node should exist");
        assert_eq!(plan.attrs.get_str("prompt"), Some("Plan for Ship feature"));
    }
}
