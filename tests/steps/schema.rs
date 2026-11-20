use std::collections::BTreeSet;

use cucumber::gherkin::Step;
use cucumber::then;
use ghostmode::host::Host;
use ghostmode::schema;
use serde_json::Value;

use crate::World;
use crate::test_host::parse_json;

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

#[then(regex = r"^the `(execute|poll)` input schema without descriptions is (\{.*\})$")]
fn input_schema_without_descriptions_is(world: &mut World, function_id: String, expected: String) {
    let mut input_schema = input_schema_of(world, &function_id);
    remove_descriptions(&mut input_schema);
    assert_eq!(input_schema, parse_json(&expected));
}

/// Removes the `description` key from the JSON schema `schema` and from each of its
/// `properties`.
fn remove_descriptions(schema: &mut Value) {
    let object = schema.as_object_mut().expect("a schema object");
    object.remove("description");
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        for property in properties.values_mut() {
            remove_descriptions(property);
        }
    }
}

/// The input schema of `execute` or `poll`, whichever is `function_id`, that `list_mcp_tools`
/// lists for the scenario's host.
fn input_schema_of(world: &World, function_id: &str) -> Value {
    let definitions = schema::list_mcp_tools(&world.host.tools());
    let definition = definitions
        .iter()
        .find(|definition| definition["name"] == function_id);
    definition.expect("a listed function")["inputSchema"].clone()
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

#[then(regex = r"^`(\S+)` is (not )?a Python identifier$")]
fn id_is_python_identifier(_world: &mut World, id: String, negation: String) {
    assert_eq!(
        schema::is_python_identifier(&id),
        negation.is_empty(),
        "{id}"
    );
}

/// The description of `execute` that `list_mcp_tools` lists for the scenario's host.
fn execute_description(world: &World) -> String {
    let tools = schema::list_mcp_tools(&world.host.tools());
    let execute = tools.iter().find(|tool| tool["name"] == "execute");
    let description = execute.expect("execute")["description"].as_str();
    description.expect("a description").to_owned()
}

/// The keys of the JSON object `value`.
fn object_keys(value: &Value) -> BTreeSet<&str> {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{value} is not an object"));
    object.keys().map(String::as_str).collect()
}
