//! Conversion between JSON values and Python values.
//!
//! Containers become arrays or objects, an `int` too big for 64 bits becomes its digit string,
//! and any other value, such as a float JSON can't hold, becomes its `repr`.

#![cfg(feature = "monty")]

use monty_types::unstable::{self, MontyNode};
use monty_types::{MontyObject, ObjectRef};
use serde_json::{Map, Number, Value};

/// The JSON the model reads for a Python value.
pub(crate) fn to_json(value: ObjectRef<'_>) -> Value {
    if let Some(items) = value.items() {
        return items.into_iter().map(to_json).collect();
    }
    if let Some(pairs) = value.pairs().filter(|_| value.type_name() == "dict") {
        let object: Map<String, Value> = pairs
            .into_iter()
            .map(|(key, value)| (to_json_key(key), to_json(value)))
            .collect();
        return Value::Object(object);
    }
    match unstable::node(value) {
        MontyNode::None => Value::Null,
        MontyNode::Bool(value) => Value::Bool(*value),
        MontyNode::Int(value) => Value::from(*value),
        MontyNode::Float(float) => {
            Number::from_f64(*float).map_or_else(|| Value::String(value.py_repr()), Value::Number)
        }
        MontyNode::String(value) | MontyNode::Repr(value) => Value::String(value.clone()),
        _ => Value::String(value.py_repr()),
    }
}

/// The Python value a script sees for a JSON value.
pub(crate) fn from_json(value: &Value) -> MontyObject {
    match value {
        Value::Null => MontyObject::none(),
        Value::Bool(value) => MontyObject::bool(*value),
        Value::Number(number) => match number.as_i64() {
            Some(int) => MontyObject::int(int),
            None => MontyObject::float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(text) => MontyObject::string(text.as_str()),
        Value::Array(items) => MontyObject::list(items.iter().map(from_json)),
        Value::Object(object) => MontyObject::dict(
            object
                .iter()
                .map(|(key, value)| (MontyObject::string(key.as_str()), from_json(value))),
        ),
    }
}

/// A dict key as a JSON object key: a `str` as it is, anything else as its JSON text.
fn to_json_key(key: ObjectRef<'_>) -> String {
    match to_json(key) {
        Value::String(text) => text,
        other => other.to_string(),
    }
}
