use std::collections::BTreeSet;

use cucumber::gherkin::Step;
use cucumber::then;
use ghostmode::schema;
use serde_json::Value;

use crate::World;
use crate::test_host::parse_json;

#[then(
    regex = r"^the MCP definitions are `execute` and `poll`, each with a name, description and inputSchema$"
)]
async fn mcp_definitions_are_execute_and_poll(world: &mut World) {
    let definitions = schema::mcp_definitions(world.sandbox_tools().await);
    let ids: Vec<&Value> = definitions
        .iter()
        .map(|definition| &definition["name"])
        .collect();
    assert_eq!(ids, ["execute", "poll"]);
    for definition in &definitions {
        assert_eq!(
            object_keys(definition),
            BTreeSet::from(["description", "inputSchema", "name"]),
            "{definition}"
        );
        assert!(definition["description"].is_string(), "{definition}");
    }
}

#[then(regex = r"^the OpenAI definitions carry the same names, descriptions and schemas$")]
async fn openai_definitions_match_mcp_definitions(world: &mut World) {
    let tools = world.sandbox_tools().await;
    let mcp_definitions = schema::mcp_definitions(tools);
    let functions = schema::openai_definitions(tools);
    assert_eq!(functions.len(), mcp_definitions.len(), "{functions:?}");
    for (function, mcp_definition) in functions.iter().zip(&mcp_definitions) {
        assert_eq!(
            object_keys(function),
            BTreeSet::from(["function", "type"]),
            "{function}"
        );
        assert_eq!(function["type"], "function", "{function}");
        let definition = &function["function"];
        assert_eq!(definition["name"], mcp_definition["name"], "{function}");
        assert_eq!(
            definition["description"], mcp_definition["description"],
            "{function}"
        );
        assert_eq!(
            definition["parameters"], mcp_definition["inputSchema"],
            "{function}"
        );
    }
}

#[then(regex = r"^the `(execute|poll)` input schema without descriptions is (\{.*\})$")]
async fn input_schema_without_descriptions_is(
    world: &mut World,
    function_id: String,
    expected: String,
) {
    let mut input_schema = input_schema_of(world, &function_id).await;
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

/// The input schema of `execute` or `poll`, whichever is `function_id`, that `mcp_definitions`
/// lists for the scenario's sandbox.
async fn input_schema_of(world: &mut World, function_id: &str) -> Value {
    let definitions = schema::mcp_definitions(world.sandbox_tools().await);
    let definition = definitions
        .iter()
        .find(|definition| definition["name"] == function_id);
    definition.expect("a listed function")["inputSchema"].clone()
}

#[then(regex = r#"^the execute description contains "(.+)"$"#)]
async fn execute_description_contains(world: &mut World, text: String) {
    let description = execute_description(world).await;
    assert!(description.contains(&text), "{description}");
}

#[then(regex = r"^the execute description contains:$")]
async fn execute_description_contains_block(world: &mut World, step: &Step) {
    let text = step.docstring.as_ref().expect("a docstring").trim();
    let description = execute_description(world).await;
    assert!(description.contains(text), "{description}");
}

#[then(regex = r#"^the execute description doesn't contain "(.+)"$"#)]
async fn execute_description_lacks(world: &mut World, text: String) {
    let description = execute_description(world).await;
    assert!(!description.contains(&text), "{description}");
}

/// The description of `execute` that `mcp_definitions` lists for the scenario's sandbox.
async fn execute_description(world: &mut World) -> String {
    let definitions = schema::mcp_definitions(world.sandbox_tools().await);
    let execute = definitions
        .iter()
        .find(|definition| definition["name"] == "execute");
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
