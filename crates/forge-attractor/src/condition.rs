use crate::{NodeOutcome, RuntimeContext};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Eq,
    Ne,
    Exists,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Clause<'a> {
    key: &'a str,
    operator: Operator,
    value: Option<&'a str>,
}

pub fn validate_condition_expression(condition: &str) -> Result<(), String> {
    for clause in parse_clauses(condition)? {
        if !is_condition_key(clause.key) {
            return Err(format!("condition key '{}' is invalid", clause.key));
        }
        if matches!(clause.operator, Operator::Eq | Operator::Ne)
            && clause.value.unwrap_or_default().trim().is_empty()
        {
            return Err(format!(
                "condition clause '{}{}' has empty value",
                clause.key,
                if clause.operator == Operator::Eq {
                    "="
                } else {
                    "!="
                }
            ));
        }
    }
    Ok(())
}

pub fn evaluate_condition_expression(
    condition: &str,
    outcome: &NodeOutcome,
    context: &RuntimeContext,
) -> Result<bool, String> {
    let clauses = parse_clauses(condition)?;
    for clause in clauses {
        let actual = resolve_key(clause.key, outcome, context)?;
        let passed = match clause.operator {
            Operator::Exists => is_truthy(actual),
            Operator::Eq => equals(actual, clause.value.unwrap_or_default()),
            Operator::Ne => !equals(actual, clause.value.unwrap_or_default()),
        };
        if !passed {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_clauses(condition: &str) -> Result<Vec<Clause<'_>>, String> {
    let mut out = Vec::new();
    for raw_clause in condition.split("&&") {
        let clause = raw_clause.trim();
        if clause.is_empty() {
            continue;
        }
        if let Some((left, right)) = clause.split_once("!=") {
            out.push(Clause {
                key: left.trim(),
                operator: Operator::Ne,
                value: Some(right.trim()),
            });
            continue;
        }
        if let Some((left, right)) = clause.split_once('=') {
            out.push(Clause {
                key: left.trim(),
                operator: Operator::Eq,
                value: Some(right.trim()),
            });
            continue;
        }
        out.push(Clause {
            key: clause,
            operator: Operator::Exists,
            value: None,
        });
    }

    for clause in &out {
        if clause.key.is_empty() {
            return Err("condition clause has empty key".to_string());
        }
    }
    Ok(out)
}

fn is_condition_key(key: &str) -> bool {
    if key == "outcome" || key == "preferred_label" {
        return true;
    }
    if !key.starts_with("context.") {
        return false;
    }
    let suffix = &key["context.".len()..];
    let mut chars = suffix.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn resolve_key<'a>(
    key: &str,
    outcome: &'a NodeOutcome,
    context: &'a RuntimeContext,
) -> Result<Option<Value>, String> {
    match key {
        "outcome" => Ok(Some(Value::String(outcome.status.as_str().to_string()))),
        "preferred_label" => Ok(outcome.preferred_label.clone().map(Value::String)),
        _ if key.starts_with("context.") => Ok(context.get(&key["context.".len()..]).cloned()),
        _ => Err(format!("condition key '{}' is invalid", key)),
    }
}

fn equals(actual: Option<Value>, expected_raw: &str) -> bool {
    let expected = parse_literal(expected_raw);
    match (actual, expected) {
        (Some(Value::String(left)), Value::String(right)) => left == right,
        (Some(Value::Bool(left)), Value::Bool(right)) => left == right,
        (Some(Value::Number(left)), Value::Number(right)) => left == right,
        (Some(left), right) => json_to_string(&left) == json_to_string(&right),
        (None, Value::Null) => true,
        (None, _) => false,
    }
}

fn parse_literal(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if trimmed.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return Value::Number(integer.into());
    }
    if let Ok(float) = trimmed.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(float) {
            return Value::Number(number);
        }
    }
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(trimmed);
    Value::String(unquoted.to_string())
}

fn json_to_string(value: &Value) -> String {
    match value {
        Value::String(inner) => inner.clone(),
        _ => value.to_string(),
    }
}

fn is_truthy(value: Option<Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(inner)) => inner,
        Some(Value::String(inner)) => !inner.is_empty(),
        Some(Value::Number(_)) => true,
        Some(Value::Array(inner)) => !inner.is_empty(),
        Some(Value::Object(inner)) => !inner.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeStatus, RuntimeContext};
    use std::collections::BTreeMap;

    fn outcome() -> NodeOutcome {
        NodeOutcome {
            status: NodeStatus::Success,
            notes: None,
            context_updates: BTreeMap::new(),
            preferred_label: Some("Yes".to_string()),
            suggested_next_ids: Vec::new(),
        }
    }

    #[test]
    fn validate_condition_expression_invalid_key_expected_err() {
        let error = validate_condition_expression("foo=bar").expect_err("validation should fail");
        assert!(error.contains("invalid"));
    }

    #[test]
    fn evaluate_condition_expression_all_clauses_match_expected_true() {
        let mut context = RuntimeContext::new();
        context.insert("ready".to_string(), Value::Bool(true));
        context.insert("tries".to_string(), Value::Number(2.into()));
        let ok = evaluate_condition_expression(
            "outcome=success && preferred_label=Yes && context.ready=true && context.tries=2",
            &outcome(),
            &context,
        )
        .expect("evaluation should succeed");
        assert!(ok);
    }

    #[test]
    fn evaluate_condition_expression_neq_clause_mismatch_expected_false() {
        let context = RuntimeContext::new();
        let ok = evaluate_condition_expression("outcome!=success", &outcome(), &context)
            .expect("evaluation should succeed");
        assert!(!ok);
    }
}
