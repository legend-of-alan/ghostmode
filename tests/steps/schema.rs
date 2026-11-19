use std::collections::BTreeSet;

use cucumber::gherkin::Step;
use cucumber::{then, when};
use ghostmode::host::Host;
use ghostmode::schema;
use serde_json::{Map, Value};

use crate::test_host::parse_json;
use crate::{Request, World};

#[then(
    regex = r"^the MCP tools are `execute` and `poll`, each with a name, description and inputSchema$"
)]
fn mcp_tools_are_execute_and_poll(world: &mut World) {
    let tools = schema::list_mcp_tools(&world.host.tools());
    let ids: Vec<&Value> = tools.iter().map(|tool| &tool["name"]).collect();
    assert_eq!(ids, ["execute", "poll"]);
    for tool in &tools {
        assert_eq!(
            object_keys(tool),
            BTreeSet::from(["description", "inputSchema", "name"]),
            "{tool}"
        );
        assert!(tool["description"].is_string(), "{tool}");
    }
}

#[then(regex = r"^the OpenAI functions carry the same names, descriptions and schemas$")]
fn openai_functions_match_mcp_tools(world: &mut World) {
    let host_tools = world.host.tools();
    let tools = schema::list_mcp_tools(&host_tools);
    let functions = schema::list_openai_functions(&host_tools);
    assert_eq!(functions.len(), tools.len(), "{functions:?}");
    for (function, tool) in functions.iter().zip(&tools) {
        assert_eq!(
            object_keys(function),
            BTreeSet::from(["function", "type"]),
            "{function}"
        );
        assert_eq!(function["type"], "function", "{function}");
        let definition = &function["function"];
        assert_eq!(definition["name"], tool["name"], "{function}");
        assert_eq!(definition["description"], tool["description"], "{function}");
        assert_eq!(definition["parameters"], tool["inputSchema"], "{function}");
    }
}

/// Sends the model's call to the sandbox. Each argument is taken out as the sandbox method
/// needs it, so a required argument the call lacks, or one the method doesn't take, fails the
/// step.
#[when(regex = r"^the model calls `(execute|poll)` with (\{.*\})$")]
async fn call_model_tool(world: &mut World, tool_id: String, args: String) {
    let mut args = parse_json(&args)
        .as_object()
        .cloned()
        .expect("an args object");
    let request = match tool_id.as_str() {
        "execute" => Request::Execute {
            code: take_arg(&mut args, "code").as_str().expect("code").into(),
            restart: args
                .remove("restart")
                .is_some_and(|restart| restart.as_bool().expect("restart")),
        },
        _ => Request::Poll(take_arg(&mut args, "run_id").as_u64().expect("run_id")),
    };
    assert!(args.is_empty(), "{tool_id} takes no {args:?}");
    world.send(request).await;
}

#[then(
    regex = r"^the `(execute|poll)` schema requires exactly `(\w+)` as an? (string|integer), with additionalProperties false$"
)]
fn model_tool_requires_exactly(world: &mut World, tool_id: String, arg: String, arg_type: String) {
    let input_schema = model_tool_input_schema(world, &tool_id);
    assert_eq!(
        input_schema["properties"][&arg]["type"],
        arg_type.as_str(),
        "{input_schema}"
    );
    assert_eq!(
        input_schema["required"],
        serde_json::json!([arg]),
        "{input_schema}"
    );
    assert_eq!(
        input_schema["additionalProperties"], false,
        "{input_schema}"
    );
}

#[then(
    regex = r"^the `(execute|poll)` schema declares `(\w+)` as a boolean, not required, defaulting to false$"
)]
fn model_tool_declares_optional_flag(world: &mut World, tool_id: String, arg: String) {
    let input_schema = model_tool_input_schema(world, &tool_id);
    let property = &input_schema["properties"][&arg];
    assert_eq!(property["type"], "boolean", "{input_schema}");
    assert_eq!(property["default"], false, "{input_schema}");
    let required = input_schema["required"].as_array().expect("required");
    assert!(!required.contains(&Value::from(arg)), "{input_schema}");
}

/// The input schema of the model's tool `tool_id` that `list_mcp_tools` lists for the
/// scenario's host.
fn model_tool_input_schema(world: &World, tool_id: &str) -> Value {
    let tools = schema::list_mcp_tools(&world.host.tools());
    let tool = tools.iter().find(|tool| tool["name"] == tool_id);
    tool.expect("a listed tool")["inputSchema"].clone()
}

#[then(regex = r#"^the execute description contains "(.+)"$"#)]
fn execute_description_contains(world: &mut World, text: String) {
    let description = execute_description(world);
    assert!(description.contains(&text), "{description}");
}

#[then(regex = r"^the execute description contains:$")]
fn execute_description_contains_block(world: &mut World, step: &Step) {
    let text = step.docstring.as_ref().expect("a docstring").trim();
    let description = execute_description(world);
    assert!(description.contains(text), "{description}");
}

#[then(regex = r#"^the execute description doesn't contain "(.+)"$"#)]
fn execute_description_lacks(world: &mut World, text: String) {
    let description = execute_description(world);
    assert!(!description.contains(&text), "{description}");
}

/// The description of the `execute` tool that `list_mcp_tools` lists for the scenario's host.
fn execute_description(world: &World) -> String {
    let tools = schema::list_mcp_tools(&world.host.tools());
    let execute = tools.iter().find(|tool| tool["name"] == "execute");
    let description = execute.expect("execute")["description"].as_str();
    description.expect("a description").to_owned()
}

/// Removes the argument `id` from the model's call, failing if the call doesn't have it.
fn take_arg(args: &mut Map<String, Value>, id: &str) -> Value {
    args.remove(id)
        .unwrap_or_else(|| panic!("the call has no `{id}`"))
}

/// The keys of the JSON object `value`.
fn object_keys(value: &Value) -> BTreeSet<&str> {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{value} is not an object"));
    object.keys().map(String::as_str).collect()
}
