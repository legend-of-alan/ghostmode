//! Conversion between JSON values and Python values, and between the host's calls and answers
//! and Monty's.
//!
//! Containers become arrays or objects, an `int` too big for 64 bits becomes its digit string,
//! and any other value, such as a float JSON can't hold, becomes its `repr`. A value fails with
//! a plain error when it is nested more than [`MAX_VALUE_DEPTH`] levels, when it needs more
//! than [`MAX_JSON_BYTES`] bytes to convert, when it holds an int with more than about
//! [`MAX_INT_DIGITS`] digits, or when two of a dict's keys become the same JSON key.

use std::collections::HashSet;
use std::fmt::{self, Write};

use monty_types::unstable::{self, MontyGraph, MontyNode, NodeId, PushValue};
use monty_types::{
    CallArgs, DEFAULT_MAX_PRINT_COLLECT_BYTES, ExcType, MontyException, MontyObject, ObjectRef,
};
use serde_json::{Map, Value};

use crate::host;

/// The most deeply a list or object may nest before conversion fails. One less
/// than the nesting serde_json allows when parsing, so a finished status or a
/// tool-args object that wraps the value can still be parsed.
const MAX_VALUE_DEPTH: usize = 126;

/// The most bytes the caller may hold for one value as JSON: a `Value` for each item and key,
/// plus the text of each string, key and `repr`. It is the same cap a script's prints get.
/// An object the value holds several times counts each time.
const MAX_JSON_BYTES: usize = DEFAULT_MAX_PRINT_COLLECT_BYTES;

/// The most digits an int may have when it becomes text, CPython's default int-to-str limit
/// (`sys.get_int_max_str_digits()`).
const MAX_INT_DIGITS: u64 = 4300;

/// The host call a script made to the tool `tool_id` with `args`, or the `TypeError` the script
/// raises when `args` hold a positional argument, or the `ValueError` it raises when the
/// arguments can't become JSON, as [`to_json`] says. All the arguments share one
/// [`MAX_JSON_BYTES`] budget.
pub(crate) fn to_call(tool_id: String, args: &CallArgs) -> Result<host::Call, MontyException> {
    if args.args().len() > 0 {
        let message = format!("{tool_id}() takes keyword arguments only");
        return Err(MontyException::new(ExcType::TypeError, Some(message)));
    }
    let mut budget = MAX_JSON_BYTES;
    let args = args
        .kwargs()
        .map(|(arg, value)| {
            let arg = arg.as_str().expect("kwargs keys are str").to_string();
            to_json_at(value, 0, &mut budget).map(|value| (arg, value))
        })
        .collect::<Result<_, _>>()
        .map_err(to_value_error)?;
    Ok(host::Call::new(tool_id, args))
}

/// The exception a host's error raises in the script: `PermissionError` for a refusal, and
/// `RuntimeError` for a failed call.
fn to_exception(error: host::Error) -> MontyException {
    match error {
        host::Error::Refused(reason) => MontyException::new(ExcType::PermissionError, Some(reason)),
        host::Error::Failed(message) => MontyException::new(ExcType::RuntimeError, Some(message)),
    }
}

/// The `ValueError` a script raises with `message`.
fn to_value_error(message: String) -> MontyException {
    MontyException::new(ExcType::ValueError, Some(message))
}

/// The Python value a script sees for a host's answer, or the exception it should raise when
/// the host failed or the answer is nested more than [`MAX_VALUE_DEPTH`] levels.
pub(crate) fn from_answer(
    answer: Result<Value, host::Error>,
) -> Result<MontyObject, MontyException> {
    match answer {
        Ok(value) => from_json(value).map_err(to_value_error),
        Err(error) => Err(to_exception(error)),
    }
}

/// The JSON the model reads for a Python value, or an error if the value is nested more than
/// [`MAX_VALUE_DEPTH`] levels, needs more than [`MAX_JSON_BYTES`] bytes to convert, holds an
/// int with more than about [`MAX_INT_DIGITS`] digits, or two of its dicts' keys become the same JSON
/// key.
pub(crate) fn to_json(value: ObjectRef<'_>) -> Result<Value, String> {
    let mut budget = MAX_JSON_BYTES;
    to_json_at(value, 0, &mut budget)
}

fn too_deep_message() -> String {
    format!(
        "the value is nested more than {MAX_VALUE_DEPTH} levels, or deeper than the sandbox's \
         recursion_depth"
    )
}

fn too_big_message() -> String {
    format!("the value needs more than {MAX_JSON_BYTES} bytes to convert to JSON")
}

/// Takes `bytes` from what is left of `budget`, or fails once it runs out.
fn spend(budget: &mut usize, bytes: usize) -> Result<(), String> {
    *budget = budget.checked_sub(bytes).ok_or_else(too_big_message)?;
    Ok(())
}

/// Writes into a string, taking each piece from what is left of `budget` and refusing the
/// piece that would run it out.
struct BudgetWriter<'b> {
    text: String,
    budget: &'b mut usize,
}

impl Write for BudgetWriter<'_> {
    fn write_str(&mut self, piece: &str) -> fmt::Result {
        spend(self.budget, piece.len()).map_err(|_| fmt::Error)?;
        self.text.push_str(piece);
        Ok(())
    }
}

/// The `repr` of `value`, taken from what is left of `budget`, or an error once it runs out or
/// if `value` holds an int with more than about [`MAX_INT_DIGITS`] digits.
fn format_repr(value: ObjectRef<'_>, budget: &mut usize) -> Result<String, String> {
    if holds_int_past_digit_limit(value) {
        return Err(format!(
            "the value holds an int with more than about {MAX_INT_DIGITS} digits"
        ));
    }
    let mut repr = BudgetWriter {
        text: String::new(),
        budget,
    };
    write!(repr, "{value}").map_err(|_| too_big_message())?;
    Ok(repr.text)
}

/// Whether `value`, or anything it holds, is an int with more than about [`MAX_INT_DIGITS`] digits.
/// Each object is checked once, however many times `value` holds it.
fn holds_int_past_digit_limit(value: ObjectRef<'_>) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        let node = unstable::node(value);
        if let MontyNode::BigInt(int) = node
            && int.bits() * 3 / 10 > MAX_INT_DIGITS
        {
            return true;
        }
        node.for_each_child(|id| {
            if seen.insert(id) {
                stack.push(unstable::child(value, id));
            }
        });
    }
    false
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

fn to_json_at(value: ObjectRef<'_>, depth: usize, budget: &mut usize) -> Result<Value, String> {
    spend(budget, size_of::<Value>())?;
    if let Some(items) = value.items() {
        if depth >= MAX_VALUE_DEPTH {
            return Err(too_deep_message());
        }
        return items
            .into_iter()
            .map(|item| to_json_at(item, depth + 1, budget))
            .collect();
    }
    if let Some(pairs) = value.pairs().filter(|_| value.type_name() == "dict") {
        if depth >= MAX_VALUE_DEPTH {
            return Err(too_deep_message());
        }
        let mut object = Map::new();
        for (key, value) in pairs {
            let key = to_json_key(key, depth + 1, budget)?;
            if object.contains_key(&key) {
                return Err(format!("the dict has the JSON key {key:?} twice"));
            }
            let value = to_json_at(value, depth + 1, budget)?;
            object.insert(key, value);
        }
        return Ok(Value::Object(object));
    }
    Ok(match unstable::node(value) {
        MontyNode::Repr(text) if text == "<deeply nested>" => return Err(too_deep_message()),
        MontyNode::String(text) | MontyNode::Repr(text) => {
            spend(budget, text.len())?;
            Value::String(text.clone())
        }
        MontyNode::None => Value::Null,
        MontyNode::Bool(value) => Value::Bool(*value),
        MontyNode::Int(value) => Value::from(*value),
        MontyNode::Float(float) if float.is_finite() => Value::from(*float),
        _ => Value::String(format_repr(value, budget)?),
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
        Value::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(int), _) => MontyNode::Int(int),
            (None, Some(int)) => return Ok(int.push_into(graph)),
            (None, None) => MontyNode::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(text) => MontyNode::String(text),
        Value::Array(items) => {
            if depth >= MAX_VALUE_DEPTH {
                drop_json(Value::Array(items));
                return Err(too_deep_message());
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
                return Err(too_deep_message());
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
fn to_json_key(key: ObjectRef<'_>, depth: usize, budget: &mut usize) -> Result<String, String> {
    match to_json_at(key, depth, budget)? {
        Value::String(text) => Ok(text),
        other => Ok(other.to_string()),
    }
}
