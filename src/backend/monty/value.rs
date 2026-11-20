//! Conversion between JSON values and Python values, and between the host's calls and answers
//! and Monty's.
//!
//! Containers become arrays or objects, an `int` too big for 64 bits becomes its digit string,
//! and any other value, such as a float JSON can't hold, becomes its `repr`. A value nested
//! more than [`MAX_VALUE_DEPTH`] levels fails with a plain error.

use monty_types::unstable::{self, MontyGraph, MontyNode, NodeId};
use monty_types::{CallArgs, ExcType, MontyException, MontyObject, ObjectRef};
use serde_json::{Map, Number, Value};

use crate::host;

/// The most deeply a list or object may nest before conversion fails. The same
/// nesting serde_json allows when parsing JSON.
const MAX_VALUE_DEPTH: usize = 127;

/// The host call a script made to the tool `tool_id` with `args`, or the `TypeError` the script
/// raises when `args` hold a positional argument, or the `ValueError` it raises when an
/// argument is nested more than [`MAX_VALUE_DEPTH`] levels.
pub(crate) fn to_call(tool_id: String, args: &CallArgs) -> Result<host::Call, MontyException> {
    if args.args().len() > 0 {
        let message = format!("{tool_id}() takes keyword arguments only");
        return Err(MontyException::new(ExcType::TypeError, Some(message)));
    }
    let args = args
        .kwargs()
        .map(|(key, value)| {
            let key = key.as_str().expect("kwargs keys are str").to_string();
            to_json(value).map(|value| (key, value))
        })
        .collect::<Result<_, _>>()
        .map_err(too_deep_exception)?;
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

/// The `ValueError` a script raises when a value is nested more than [`MAX_VALUE_DEPTH`]
/// levels.
fn too_deep_exception(message: String) -> MontyException {
    MontyException::new(ExcType::ValueError, Some(message))
}

/// The Python value a script sees for a host's answer, or the exception it should raise when
/// the host failed or the answer is nested more than [`MAX_VALUE_DEPTH`] levels.
pub(crate) fn from_answer(
    answer: Result<Value, host::Error>,
) -> Result<MontyObject, MontyException> {
    match answer {
        Ok(value) => from_json(value).map_err(too_deep_exception),
        Err(error) => Err(to_exception(error)),
    }
}

/// The JSON the model reads for a Python value, or an error if the value is nested more than
/// [`MAX_VALUE_DEPTH`] levels.
pub(crate) fn to_json(value: ObjectRef<'_>) -> Result<Value, String> {
    to_json_at(value, 0)
}

fn too_deep() -> String {
    format!("the value is nested more than {MAX_VALUE_DEPTH} levels")
}

/// Drops a JSON value by walking it on the heap, so a value nested more than
/// the native stack can hold doesn't overflow when it is discarded.
fn drop_json(value: Value) {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(object) => stack.extend(object.into_iter().map(|(_, value)| value)),
            _ => {}
        }
    }
}

fn to_json_at(value: ObjectRef<'_>, depth: usize) -> Result<Value, String> {
    if let Some(items) = value.items() {
        if depth >= MAX_VALUE_DEPTH {
            return Err(too_deep());
        }
        return items
            .into_iter()
            .map(|item| to_json_at(item, depth + 1))
            .collect();
    }
    if let Some(pairs) = value.pairs().filter(|_| value.type_name() == "dict") {
        if depth >= MAX_VALUE_DEPTH {
            return Err(too_deep());
        }
        let object: Result<Map<String, Value>, _> = pairs
            .into_iter()
            .map(|(key, value)| Ok((to_json_key(key, depth + 1)?, to_json_at(value, depth + 1)?)))
            .collect();
        return object.map(Value::Object);
    }
    Ok(match unstable::node(value) {
        MontyNode::None => Value::Null,
        MontyNode::Bool(value) => Value::Bool(*value),
        MontyNode::Int(value) => Value::from(*value),
        MontyNode::Float(float) => {
            Number::from_f64(*float).map_or_else(|| Value::String(value.py_repr()), Value::Number)
        }
        MontyNode::String(value) | MontyNode::Repr(value) => Value::String(value.clone()),
        _ => Value::String(value.py_repr()),
    })
}

/// The Python value a script sees for a JSON value, built in one arena, or an error if the
/// value is nested more than [`MAX_VALUE_DEPTH`] levels.
fn from_json(value: Value) -> Result<MontyObject, String> {
    let mut graph = MontyGraph::new();
    let root = push_json(value, &mut graph, 0)?;
    Ok(unstable::object_from_graph(graph, root)
        .expect("push_json returns the id of a node it pushed"))
}

/// Pushes a JSON value's nodes onto `graph`, children first, and returns the id of its root.
fn push_json(value: Value, graph: &mut MontyGraph, depth: usize) -> Result<NodeId, String> {
    let node = match value {
        Value::Null => MontyNode::None,
        Value::Bool(value) => MontyNode::Bool(value),
        Value::Number(number) => match number.as_i64() {
            Some(int) => MontyNode::Int(int),
            None => MontyNode::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(text) => MontyNode::String(text),
        Value::Array(items) => {
            if depth >= MAX_VALUE_DEPTH {
                drop_json(Value::Array(items));
                return Err(too_deep());
            }
            let mut ids = Vec::with_capacity(items.len());
            let mut items = items.into_iter();
            while let Some(item) = items.next() {
                match push_json(item, graph, depth + 1) {
                    Ok(id) => ids.push(id),
                    Err(error) => {
                        drop_json(Value::Array(items.collect()));
                        return Err(error);
                    }
                }
            }
            MontyNode::List(ids)
        }
        Value::Object(object) => {
            if depth >= MAX_VALUE_DEPTH {
                drop_json(Value::Object(object));
                return Err(too_deep());
            }
            let mut pairs = Vec::with_capacity(object.len());
            let mut object = object.into_iter();
            while let Some((key, value)) = object.next() {
                let key = graph.push(MontyNode::String(key));
                match push_json(value, graph, depth + 1) {
                    Ok(value) => pairs.push((key, value)),
                    Err(error) => {
                        drop_json(Value::Object(object.collect()));
                        return Err(error);
                    }
                }
            }
            MontyNode::Dict(pairs)
        }
    };
    Ok(graph.push(node))
}

/// A dict key as a JSON object key: a `str` as it is, anything else as its JSON text.
fn to_json_key(key: ObjectRef<'_>, depth: usize) -> Result<String, String> {
    match to_json_at(key, depth)? {
        Value::String(text) => Ok(text),
        other => Ok(other.to_string()),
    }
}
