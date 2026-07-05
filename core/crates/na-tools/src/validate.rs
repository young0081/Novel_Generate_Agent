//! A small, dependency-light JSON-Schema validator.
//!
//! Tool arguments arrive from the model as arbitrary JSON; before a tool runs we
//! check them against the tool's declared [`ToolSpec::input_schema`](crate::ToolSpec).
//! This validator supports the practical subset of JSON Schema we actually use:
//!
//! * `type` — `"object"`, `"string"`, `"number"`, `"integer"`, `"boolean"`,
//!   `"array"`, `"null"` (a single string or an array of allowed types).
//! * `required` — array of property names that must be present.
//! * `properties` — per-property subschemas (validated recursively).
//! * `items` — subschema applied to every array element.
//! * `enum` — value must equal one of the listed JSON values.
//! * `minimum` / `maximum` — numeric bounds (inclusive).
//! * `minLength` / `maxLength` — string length bounds (in Unicode scalar values).
//! * `pattern` — a regular expression the string must (partially) match.
//! * `additionalProperties` — when `false`, properties not in `properties` are
//!   rejected.
//!
//! On failure we return a [`CoreError::invalid_input`] whose message names the
//! offending JSON-path-ish location (e.g. `args.title: expected string`). A
//! schema that is itself malformed (e.g. `pattern` is not a valid regex) is
//! reported as an `invalid_input` error too, since the validation cannot proceed.

use na_common::{CoreError, Json, Result};

/// Validate `args` against `schema`, returning `Ok(())` if it conforms.
///
/// The top-level path label is `"args"`; nested locations are reported as
/// `args.field`, `args.list[0]`, etc.
pub fn validate(schema: &Json, args: &Json) -> Result<()> {
    validate_at("args", schema, args)
}

/// Recursive worker: validate `value` at `path` against `schema`.
fn validate_at(path: &str, schema: &Json, value: &Json) -> Result<()> {
    // A schema must be an object (we treat anything else as "accept all", which
    // matches the permissive `true` schema in JSON Schema). A boolean `true`
    // schema accepts everything; `false` rejects everything.
    let obj = match schema {
        Json::Object(o) => o,
        Json::Bool(true) => return Ok(()),
        Json::Bool(false) => {
            return Err(err(path, "schema forbids any value here"));
        }
        // Non-object, non-bool schema: nothing to check.
        _ => return Ok(()),
    };

    // ---- type ----
    if let Some(type_node) = obj.get("type") {
        check_type(path, type_node, value)?;
    }

    // ---- enum ----
    if let Some(Json::Array(allowed)) = obj.get("enum") {
        if !allowed.iter().any(|a| a == value) {
            return Err(err(
                path,
                &format!(
                    "value {} is not one of the allowed enum values",
                    compact(value)
                ),
            ));
        }
    }

    // ---- numeric bounds ----
    if let Some(min) = obj.get("minimum").and_then(Json::as_f64) {
        if let Some(n) = value.as_f64() {
            if n < min {
                return Err(err(path, &format!("value {n} is below minimum {min}")));
            }
        }
    }
    if let Some(max) = obj.get("maximum").and_then(Json::as_f64) {
        if let Some(n) = value.as_f64() {
            if n > max {
                return Err(err(path, &format!("value {n} is above maximum {max}")));
            }
        }
    }

    // ---- string constraints ----
    if let Some(s) = value.as_str() {
        if let Some(min_len) = obj.get("minLength").and_then(Json::as_u64) {
            let len = s.chars().count() as u64;
            if len < min_len {
                return Err(err(
                    path,
                    &format!("string length {len} is below minLength {min_len}"),
                ));
            }
        }
        if let Some(max_len) = obj.get("maxLength").and_then(Json::as_u64) {
            let len = s.chars().count() as u64;
            if len > max_len {
                return Err(err(
                    path,
                    &format!("string length {len} is above maxLength {max_len}"),
                ));
            }
        }
        if let Some(Json::String(pat)) = obj.get("pattern") {
            let re = regex::Regex::new(pat)
                .map_err(|e| err(path, &format!("schema has invalid pattern {pat:?}: {e}")))?;
            if !re.is_match(s) {
                return Err(err(
                    path,
                    &format!("string {s:?} does not match pattern {pat:?}"),
                ));
            }
        }
    }

    // ---- object constraints ----
    if let Some(Json::Object(value_obj)) = Some(value).filter(|v| v.is_object()) {
        // required
        if let Some(Json::Array(req)) = obj.get("required") {
            for r in req {
                if let Some(name) = r.as_str() {
                    if !value_obj.contains_key(name) {
                        return Err(err(path, &format!("missing required property {name:?}")));
                    }
                }
            }
        }

        let properties = obj.get("properties").and_then(Json::as_object);

        // additionalProperties: false => reject unknown keys.
        let additional_allowed = match obj.get("additionalProperties") {
            Some(Json::Bool(b)) => *b,
            // A subschema for additionalProperties is treated as "allowed" here
            // (we validate known properties only); default is allowed.
            _ => true,
        };
        if !additional_allowed {
            if let Some(props) = properties {
                for key in value_obj.keys() {
                    if !props.contains_key(key) {
                        return Err(err(
                            path,
                            &format!("additional property {key:?} is not allowed"),
                        ));
                    }
                }
            } else {
                // No properties defined and additionalProperties:false => no keys allowed.
                if let Some(key) = value_obj.keys().next() {
                    return Err(err(
                        path,
                        &format!("additional property {key:?} is not allowed"),
                    ));
                }
            }
        }

        // Validate each declared property that is present.
        if let Some(props) = properties {
            for (name, subschema) in props {
                if let Some(child) = value_obj.get(name) {
                    let child_path = format!("{path}.{name}");
                    validate_at(&child_path, subschema, child)?;
                }
            }
        }
    }

    // ---- array items ----
    if let Some(items_schema) = obj.get("items") {
        if let Some(arr) = value.as_array() {
            for (i, elem) in arr.iter().enumerate() {
                let child_path = format!("{path}[{i}]");
                validate_at(&child_path, items_schema, elem)?;
            }
        }
    }

    Ok(())
}

/// Check `value` against a `type` node, which is either a string or an array of
/// strings (any of which is acceptable).
fn check_type(path: &str, type_node: &Json, value: &Json) -> Result<()> {
    let matches_one = |t: &str| type_matches(t, value);

    match type_node {
        Json::String(t) => {
            if matches_one(t) {
                Ok(())
            } else {
                Err(err(
                    path,
                    &format!("expected type {t}, got {}", json_type_name(value)),
                ))
            }
        }
        Json::Array(types) => {
            let names: Vec<&str> = types.iter().filter_map(Json::as_str).collect();
            if names.iter().any(|t| matches_one(t)) {
                Ok(())
            } else {
                Err(err(
                    path,
                    &format!(
                        "expected one of types {names:?}, got {}",
                        json_type_name(value)
                    ),
                ))
            }
        }
        // Non-standard type node: don't enforce.
        _ => Ok(()),
    }
}

/// Does `value` satisfy the JSON-Schema `type` named `t`?
fn type_matches(t: &str, value: &Json) -> bool {
    match t {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        // An integer must be a number with no fractional part.
        "integer" => value.is_i64() || value.is_u64() || is_integral_float(value),
        // Unknown type name: accept (we don't enforce types we don't model).
        _ => true,
    }
}

/// Is `value` a float that happens to be integral (e.g. `3.0`)?
fn is_integral_float(value: &Json) -> bool {
    matches!(value.as_f64(), Some(f) if f.fract() == 0.0)
}

/// A short human name for a JSON value's type (for error messages).
fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

/// Build an `invalid_input` error tagged with the JSON path.
fn err(path: &str, message: &str) -> CoreError {
    CoreError::invalid_input(format!("{path}: {message}"))
}

/// Compactly render a JSON value for an error message (truncated if huge).
fn compact(value: &Json) -> String {
    let s = value.to_string();
    if s.len() > 80 {
        format!("{}…", &s[..80])
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::json;

    #[test]
    fn accepts_matching_object() {
        let schema = json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": { "type": "string", "minLength": 1 },
                "age": { "type": "integer", "minimum": 0, "maximum": 120 }
            }
        });
        let args = json!({ "name": "Lin", "age": 17 });
        assert!(validate(&schema, &args).is_ok());
    }

    #[test]
    fn rejects_missing_required() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string" } }
        });
        let err = validate(&schema, &json!({})).unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
        assert!(err.message.contains("missing required property"));
        assert!(err.message.contains("name"));
    }

    #[test]
    fn rejects_wrong_type() {
        let schema = json!({ "type": "object", "properties": { "n": { "type": "integer" } } });
        let err = validate(&schema, &json!({ "n": "not a number" })).unwrap_err();
        assert!(err.message.contains("args.n"));
        assert!(err.message.contains("expected type integer"));
    }

    #[test]
    fn integer_accepts_integral_float_rejects_fraction() {
        let schema = json!({ "type": "integer" });
        assert!(validate(&schema, &json!(3)).is_ok());
        assert!(validate(&schema, &json!(3.0)).is_ok());
        assert!(validate(&schema, &json!(3.5)).is_err());
    }

    #[test]
    fn numeric_bounds() {
        let schema = json!({ "type": "number", "minimum": 1, "maximum": 5 });
        assert!(validate(&schema, &json!(1)).is_ok());
        assert!(validate(&schema, &json!(5)).is_ok());
        assert!(validate(&schema, &json!(0.9)).is_err());
        assert!(validate(&schema, &json!(5.1)).is_err());
    }

    #[test]
    fn string_length_and_pattern() {
        let schema = json!({
            "type": "string",
            "minLength": 2,
            "maxLength": 4,
            "pattern": "^[a-z]+$"
        });
        assert!(validate(&schema, &json!("abc")).is_ok());
        assert!(validate(&schema, &json!("a")).is_err()); // too short
        assert!(validate(&schema, &json!("abcde")).is_err()); // too long
        assert!(validate(&schema, &json!("AB")).is_err()); // pattern
    }

    #[test]
    fn min_length_counts_unicode_scalars() {
        // CJK: 2 chars but 6 bytes. minLength is in chars.
        let schema = json!({ "type": "string", "minLength": 2, "maxLength": 2 });
        assert!(validate(&schema, &json!("龙王")).is_ok());
        assert!(validate(&schema, &json!("龙")).is_err());
    }

    #[test]
    fn enum_constraint() {
        let schema = json!({ "enum": ["anchor", "full", "structured"] });
        assert!(validate(&schema, &json!("full")).is_ok());
        let err = validate(&schema, &json!("nope")).unwrap_err();
        assert!(err.message.contains("not one of the allowed enum"));
    }

    #[test]
    fn nested_object_paths_reported() {
        let schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": { "email": { "type": "string", "pattern": "@" } }
                }
            }
        });
        let err = validate(&schema, &json!({ "user": { "email": "no-at-sign" } })).unwrap_err();
        assert!(err.message.contains("args.user.email"), "{}", err.message);
    }

    #[test]
    fn array_items_validated_with_index() {
        let schema = json!({
            "type": "array",
            "items": { "type": "integer", "minimum": 0 }
        });
        assert!(validate(&schema, &json!([0, 1, 2])).is_ok());
        let err = validate(&schema, &json!([0, -1, 2])).unwrap_err();
        assert!(err.message.contains("args[1]"), "{}", err.message);
        assert!(err.message.contains("below minimum"));
    }

    #[test]
    fn array_of_objects() {
        let schema = json!({
            "type": "array",
            "items": {
                "type": "object",
                "required": ["k"],
                "properties": { "k": { "type": "string" } }
            }
        });
        assert!(validate(&schema, &json!([{ "k": "a" }, { "k": "b" }])).is_ok());
        let err = validate(&schema, &json!([{ "k": "a" }, { }])).unwrap_err();
        assert!(err.message.contains("args[1]"), "{}", err.message);
    }

    #[test]
    fn additional_properties_false_rejects_unknown() {
        let schema = json!({
            "type": "object",
            "properties": { "a": { "type": "string" } },
            "additionalProperties": false
        });
        assert!(validate(&schema, &json!({ "a": "x" })).is_ok());
        let err = validate(&schema, &json!({ "a": "x", "b": 1 })).unwrap_err();
        assert!(err.message.contains("additional property"));
        assert!(err.message.contains("\"b\""));
    }

    #[test]
    fn additional_properties_true_allows_unknown() {
        let schema = json!({
            "type": "object",
            "properties": { "a": { "type": "string" } },
            "additionalProperties": true
        });
        assert!(validate(&schema, &json!({ "a": "x", "b": 1 })).is_ok());
    }

    #[test]
    fn type_array_allows_multiple() {
        let schema = json!({ "type": ["string", "null"] });
        assert!(validate(&schema, &json!("x")).is_ok());
        assert!(validate(&schema, &json!(null)).is_ok());
        assert!(validate(&schema, &json!(5)).is_err());
    }

    #[test]
    fn boolean_schema_true_and_false() {
        assert!(validate(&json!(true), &json!({ "anything": 1 })).is_ok());
        assert!(validate(&json!(false), &json!(1)).is_err());
    }

    #[test]
    fn invalid_regex_in_schema_is_invalid_input() {
        let schema = json!({ "type": "string", "pattern": "[" });
        let err = validate(&schema, &json!("x")).unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
        assert!(err.message.contains("invalid pattern"));
    }
}
