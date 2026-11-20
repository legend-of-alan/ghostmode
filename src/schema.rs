//! The two functions the model calls, `execute` and `poll`, with their JSON Schemas, in MCP's
//! and OpenAI's shapes. The caller hands one of the two lists to its model.

use serde_json::{Value, json};

use crate::host;

/// The two functions the model calls, `execute` then `poll`, in the shape MCP's `tools/list`
/// answers with: `{"name", "description", "inputSchema"}`. The schemas set
/// `additionalProperties: false`, and the execute description lists each of `host_tools` whose
/// id is a Python identifier as a typed, keyword-only Python stub.
pub fn mcp_definitions(host_tools: &[host::Tool]) -> Vec<Value> {
    let stubs: Vec<String> = host_tools
        .iter()
        .filter(|tool| is_python_identifier(&tool.id))
        .map(format_stub)
        .collect();
    let execute_description = format!(
        "Runs a Python script in the sandbox's session and answers with the value of its last \
         expression as {{\"status\": \"finished\", \"value\": ...}}, or {{\"status\": \"failed\", \
         \"error\": ..., \"calls\": [...]}}. Globals persist from one script to the next. A \
         script still waiting on a tool when the wait ends answers {{\"status\": \"running\", \
         \"run_id\": ...}}: poll that run id for the rest.\n\nThe script calls these tools as \
         Python functions, which take keyword arguments only:\n\n{}",
        stubs.join("\n\n")
    );
    vec![
        json!({
            "name": "execute",
            "description": execute_description,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {"type": "string", "description": "The Python script to run."},
                    "restart": {
                        "type": "boolean",
                        "default": false,
                        "description": "Start the session fresh first, with no globals. The status then carries \"restarted\": true.",
                    },
                },
                "required": ["code"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "poll",
            "description": "Waits for the run that execute left running, and answers as execute does.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": {"type": "integer", "description": "The run id of a running status."},
                },
                "required": ["run_id"],
                "additionalProperties": false,
            },
        }),
    ]
}

/// The same two functions as [`mcp_definitions`], as OpenAI functions:
/// `{"type": "function", "function": {"name", "description", "parameters"}}`.
pub fn openai_definitions(host_tools: &[host::Tool]) -> Vec<Value> {
    mcp_definitions(host_tools)
        .into_iter()
        .map(|definition| {
            json!({
                "type": "function",
                "function": {
                    "name": definition["name"],
                    "description": definition["description"],
                    "parameters": definition["inputSchema"],
                },
            })
        })
        .collect()
}

/// How a script calls `tool`, as a Python stub with the tool's description as its docstring:
/// `def id(*, arg: str, opt: int | None = None) -> Any:`. The arguments are its input schema's
/// `properties`, the `required` ones first and then the rest, each alphabetical, and the rest
/// default to `None`. A tool with no properties is `def id() -> Any:`.
fn format_stub(tool: &host::Tool) -> String {
    let required = tool.input_schema["required"].as_array();
    let is_required = |arg: &str| required.is_some_and(|ids| ids.iter().any(|id| id == arg));
    let mut args: Vec<(&String, &Value)> = tool.input_schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .collect();
    args.sort_unstable_by_key(|(arg, _)| (!is_required(arg), *arg));
    let args: Vec<String> = args
        .into_iter()
        .map(|(arg, arg_schema)| {
            let python_type = to_python_type(arg_schema);
            if is_required(arg) {
                format!("{arg}: {python_type}")
            } else {
                format!("{arg}: {python_type} | None = None")
            }
        })
        .collect();
    let args = if args.is_empty() {
        String::new()
    } else {
        format!("*, {}", args.join(", "))
    };
    let mut docstring = tool.description.replace('\\', r"\\");
    if docstring.contains(r#"""""#) || docstring.ends_with('"') {
        docstring = docstring.replace('"', r#"\""#);
    }
    let docstring = docstring.replace('\n', "\n    ");
    format!(
        "def {}({args}) -> Any:\n    \"\"\"{docstring}\"\"\"",
        tool.id
    )
}

/// The Python type of a value that the JSON Schema `value_schema` accepts: `str`, `int`,
/// `float`, `bool`, `list[...]`, `dict[str, Any]` or, for an `enum`, `Literal[...]`. Any other
/// schema is `Any`.
fn to_python_type(value_schema: &Value) -> String {
    if let Some(values) = value_schema["enum"].as_array() {
        let literals: Vec<String> = values.iter().map(to_python_literal).collect();
        return format!("Literal[{}]", literals.join(", "));
    }
    match value_schema["type"].as_str() {
        Some("string") => "str".to_owned(),
        Some("integer") => "int".to_owned(),
        Some("number") => "float".to_owned(),
        Some("boolean") => "bool".to_owned(),
        Some("array") => format!("list[{}]", to_python_type(&value_schema["items"])),
        Some("object") => "dict[str, Any]".to_owned(),
        _ => "Any".to_owned(),
    }
}

/// The JSON value `value` as a Python literal: `True`, `False` and `None` for JSON's `true`,
/// `false` and `null`, and its JSON text otherwise.
fn to_python_literal(value: &Value) -> String {
    match value {
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        _ => value.to_string(),
    }
}

/// The words Python reserves, which a script can't call as a function.
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Whether a script can call a function named `id`: `id` is ASCII letters, digits and `_`,
/// doesn't start with a digit, and isn't a Python keyword. A host can check its tool ids with it.
pub fn is_python_identifier(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with(|c: char| c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !PYTHON_KEYWORDS.contains(&id)
}
