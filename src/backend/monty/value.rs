//! Conversion between JSON values and Python values.
//!
//! Containers become arrays or objects, an `int` too big for 64 bits becomes its digit string,
//! and any other value, such as a float JSON can't hold, becomes its `repr`.

use monty_types::unstable::{self, MontyGraph, MontyNode, NodeId};
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

/// The Python value a script sees for a JSON value, built in one arena.
pub(crate) fn from_json(value: &Value) -> MontyObject {
    let mut graph = MontyGraph::new();
    let root = push_json(value, &mut graph);
    unstable::object_from_graph(graph, root).expect("push_json returns the id of a node it pushed")
}

/// Pushes a JSON value's nodes onto `graph`, children first, and returns the id of its root.
fn push_json(value: &Value, graph: &mut MontyGraph) -> NodeId {
    let node = match value {
        Value::Null => MontyNode::None,
        Value::Bool(value) => MontyNode::Bool(*value),
        Value::Number(number) => match number.as_i64() {
            Some(int) => MontyNode::Int(int),
            None => MontyNode::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(text) => MontyNode::String(text.clone()),
        Value::Array(items) => {
            MontyNode::List(items.iter().map(|item| push_json(item, graph)).collect())
        }
        Value::Object(object) => MontyNode::Dict(
            object
                .iter()
                .map(|(key, value)| {
                    let key = graph.push(MontyNode::String(key.clone()));
                    (key, push_json(value, graph))
                })
                .collect(),
        ),
    };
    graph.push(node)
}

/// A dict key as a JSON object key: a `str` as it is, anything else as its JSON text.
fn to_json_key(key: ObjectRef<'_>) -> String {
    match to_json(key) {
        Value::String(text) => text,
        other => other.to_string(),
    }
}
