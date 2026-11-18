//! The model's two tools, `execute` and `poll`, with their JSON Schemas, as MCP tools and as
//! OpenAI functions. The caller hands one of the two lists to its model.

use serde_json::{Value, json};

use crate::host;

/// The model's `execute` and `poll` tools, in that order, in the shape MCP's `tools/list`
/// answers with: `{"name", "description", "inputSchema"}`. The schemas set
/// `additionalProperties: false`, and the execute description lists each of `host_tools` as a
/// keyword-only signature.
pub fn list_mcp_tools(host_tools: &[host::Tool]) -> Vec<Value> {
    let signatures: Vec<String> = host_tools.iter().map(format_signature).collect();
    let execute_description = format!(
        "Runs a Python script in the sandbox's session and answers with the value of its last \
         expression as {{\"status\": \"finished\", \"value\": ...}}, or {{\"status\": \"failed\", \
         \"error\": ..., \"calls\": [...]}}. Globals persist from one script to the next. A \
         script still waiting on a tool when the wait ends answers {{\"status\": \"running\", \
         \"run_id\": ...}}: poll that run id for the rest.\n\nThe script calls these tools as \
         Python functions, which take keyword arguments only:\n{}",
        signatures.join("\n")
    );
    vec![
        json!({
            "name": "execute",
            "description": execute_description,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {"type": "string", "description": "The Python script to run."},
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

/// The same tools as [`list_mcp_tools`], as OpenAI function tools:
/// `{"type": "function", "function": {"name", "description", "parameters"}}`. Strict mode
/// accepts their schemas.
pub fn list_openai_functions(host_tools: &[host::Tool]) -> Vec<Value> {
    list_mcp_tools(host_tools)
        .into_iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": tool["description"],
                    "parameters": tool["inputSchema"],
                },
            })
        })
        .collect()
}

/// How a script calls `tool`: `id(*, arg, ...)`, with the arguments of its input schema's
/// `properties`, the `required` ones first and then the rest, each alphabetical. A tool with
/// no properties is `id()`.
fn format_signature(tool: &host::Tool) -> String {
    let required = tool.input_schema["required"].as_array();
    let is_required = |arg: &&str| required.is_some_and(|ids| ids.iter().any(|id| id == *arg));
    let mut args: Vec<&str> = tool.input_schema["properties"]
        .as_object()
        .into_iter()
        .flat_map(|properties| properties.keys().map(String::as_str))
        .collect();
    args.sort_unstable_by_key(|arg| (!is_required(arg), *arg));
    if args.is_empty() {
        format!("{}()", tool.id)
    } else {
        format!("{}(*, {})", tool.id, args.join(", "))
    }
}
