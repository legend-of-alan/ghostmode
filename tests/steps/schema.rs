use std::collections::BTreeSet;

use cucumber::{then, when};
use ghostmode::host::Host;
use ghostmode::schema;
use serde_json::{Map, Value, json};

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
        let keys: BTreeSet<&str> = tool
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["description", "inputSchema", "name"]),
            "{tool}"
        );
        assert!(tool["description"].is_string(), "{tool}");
    }
}

#[then(regex = r"^the OpenAI functions carry the same names, descriptions and schemas$")]
fn openai_functions_match_mcp_tools(world: &mut World) {
    let host_tools = world.host.tools();
    let functions: Vec<Value> = schema::list_mcp_tools(&host_tools)
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
        .collect();
    assert_eq!(schema::list_openai_functions(&host_tools), functions);
}

/// Checks `args` against the MCP input schema of `tool_id`, as a strict-mode client would,
/// then sends the call. Each argument is taken out as the sandbox method needs it, so an
/// argument the schema declares but the method doesn't take, or the reverse, fails the step.
#[when(regex = r"^the model calls `(execute|poll)` with (\{.*\})$")]
async fn call_model_tool(world: &mut World, tool_id: String, args: String) {
    let tools = schema::list_mcp_tools(&world.host.tools());
    let tool = tools.iter().find(|tool| tool["name"] == tool_id.as_str());
    let input_schema = &tool.expect("a listed tool")["inputSchema"];
    assert_eq!(
        input_schema["additionalProperties"], false,
        "{input_schema}"
    );
    let properties = input_schema["properties"].as_object().expect("properties");
    let declared: BTreeSet<&String> = properties.keys().collect();
    let required: BTreeSet<&str> = input_schema["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|id| id.as_str().expect("a property id"))
        .collect();
    assert_eq!(
        required,
        declared.iter().map(|id| id.as_str()).collect(),
        "{input_schema}"
    );
    let mut args = parse_json(&args)
        .as_object()
        .cloned()
        .expect("an args object");
    assert_eq!(
        args.keys().collect::<BTreeSet<_>>(),
        declared,
        "{input_schema}"
    );
    for (id, value) in &args {
        let matches_type = match properties[id]["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.is_u64(),
            other => panic!("{id} has the unexpected type {other:?}"),
        };
        assert!(matches_type, "{id} is not {}", properties[id]["type"]);
    }
    let request = match tool_id.as_str() {
        "execute" => Request::Execute(take_arg(&mut args, "code").as_str().expect("code").into()),
        _ => Request::Poll(take_arg(&mut args, "run_id").as_u64().expect("run_id")),
    };
    assert!(args.is_empty(), "{tool_id} takes no {args:?}");
    world.send(request).await;
}

#[then(regex = r#"^the execute description contains "(.+)"$"#)]
fn execute_description_contains(world: &mut World, text: String) {
    let tools = schema::list_mcp_tools(&world.host.tools());
    let execute = tools.iter().find(|tool| tool["name"] == "execute");
    let description = execute.expect("execute")["description"]
        .as_str()
        .expect("a description");
    assert!(description.contains(&text), "{description}");
}

/// Removes the argument `id` from `args`, failing if the schema didn't declare it.
fn take_arg(args: &mut Map<String, Value>, id: &str) -> Value {
    args.remove(id)
        .unwrap_or_else(|| panic!("the schema does not declare `{id}`"))
}
