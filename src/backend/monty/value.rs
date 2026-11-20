//! Conversion between JSON values and Python values, and between the host's calls and answers
//! and Monty's.
//!
//! Containers become arrays or objects, an `int` too big for 64 bits becomes its digit string,
//! and any other value, such as a float JSON can't hold, becomes its `repr`.

use monty_types::unstable::{self, MontyGraph, MontyNode, NodeId};
use monty_types::{CallArgs, ExcType, MontyException, MontyObject, ObjectRef};
use serde_json::{Map, Number, Value};

use crate::host;

/// The host call a script made to the tool `tool_id` with `args`, or the `TypeError` the script
/// raises when `args` hold a positional argument.
pub(crate) fn to_call(tool_id: String, args: &CallArgs) -> Result<host::Call, MontyException> {
    if args.args().len() > 0 {
        let message = format!("{tool_id}() takes keyword arguments only");
        return Err(MontyException::new(ExcType::TypeError, Some(message)));
    }
    let args = args.kwargs().map(|(key, value)| {
        let key = key.as_str().expect("kwargs keys are str").to_string();
        (key, to_json(value))
    });
    let args = args.collect();
    Ok(host::Call::new(tool_id, args))
}

/// The exception a host's error raises in the script: `PermissionError` for a refusal, and
/// `RuntimeError` for a failed call.
pub(crate) fn to_exception(error: host::Error) -> MontyException {
    match error {
        host::Error::Refused(reason) => MontyException::new(ExcType::PermissionError, Some(reason)),
        host::Error::Failed(message) => MontyException::new(ExcType::RuntimeError, Some(message)),
    }
}

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
pub(crate) fn from_json(value: Value) -> MontyObject {
    let mut graph = MontyGraph::new();
    let root = push_json(value, &mut graph);
    unstable::object_from_graph(graph, root).expect("push_json returns the id of a node it pushed")
}

/// Pushes a JSON value's nodes onto `graph`, children first, and returns the id of its root.
fn push_json(value: Value, graph: &mut MontyGraph) -> NodeId {
    let node = match value {
        Value::Null => MontyNode::None,
        Value::Bool(value) => MontyNode::Bool(value),
        Value::Number(number) => match number.as_i64() {
            Some(int) => MontyNode::Int(int),
            None => MontyNode::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(text) => MontyNode::String(text),
        Value::Array(items) => MontyNode::List(
            items
                .into_iter()
                .map(|item| push_json(item, graph))
                .collect(),
        ),
        Value::Object(object) => MontyNode::Dict(
            object
                .into_iter()
                .map(|(key, value)| {
                    let key = graph.push(MontyNode::String(key));
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
