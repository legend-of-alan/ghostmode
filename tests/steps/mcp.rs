use std::collections::HashMap;
use std::sync::Arc;

use cucumber::{given, then, when};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::{RoleClient, RoleServer, ServerHandler};
use serde_json::{Map, Value};

use crate::World;

/// What a tool on the test MCP server answers, from the server to the MCP client.
#[derive(Clone)]
enum Answer {
    /// The call's arguments as structured content, beside text that is not JSON.
    ArgumentsAsStructuredContent,
    /// The call's arguments as JSON text, with no structured content.
    ArgumentsAsText,
    /// `isError: true` with this text.
    Error(String),
}

/// An in-process MCP server whose tools answer as the scenario's Given steps set up.
#[derive(Clone, Default)]
pub(crate) struct TestServer {
    /// Each tool's answer, by MCP id.
    answers: HashMap<String, Answer>,
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.answers.keys().map(|mcp_id| {
            let schema = Map::from_iter([("type".to_owned(), Value::from("object"))]);
            Tool::new(
                mcp_id.clone(),
                format!("The scenario's {mcp_id} tool."),
                schema,
            )
        });
        Ok(ListToolsResult::with_all_items(tools.collect()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = match &self.answers[request.name.as_ref()] {
            Answer::ArgumentsAsStructuredContent => {
                let mut result = CallToolResult::success(vec![ContentBlock::text("not JSON")]);
                result.structured_content = Some(arguments);
                result
            }
            Answer::ArgumentsAsText => {
                CallToolResult::success(vec![ContentBlock::text(arguments.to_string())])
            }
            Answer::Error(text) => CallToolResult::error(vec![ContentBlock::text(text.clone())]),
        };
        Ok(CallToolResponse::Complete(result))
    }
}

/// The scenario's MCP server, and both ends of its connection once a When step connects.
#[derive(Default)]
pub(crate) struct Connection {
    server: TestServer,
    /// The server and client services, kept running for the rest of the scenario.
    services: Option<(
        RunningService<RoleServer, TestServer>,
        RunningService<RoleClient, ()>,
    )>,
    /// What `mcp::Server::new` returned as its error, if it refused the server's tools.
    client_error: Option<String>,
}

#[given(regex = r"^an MCP server whose `(\S+)` answers its arguments as structured content$")]
fn server_answers_structured(world: &mut World, mcp_id: String) {
    let answers = &mut world.mcp.server.answers;
    answers.insert(mcp_id, Answer::ArgumentsAsStructuredContent);
}

#[given(
    regex = r"^(?:an MCP server whose|the MCP server's) `(\S+)` (?:also )?answers its arguments as text$"
)]
fn server_answers_text(world: &mut World, mcp_id: String) {
    let answers = &mut world.mcp.server.answers;
    answers.insert(mcp_id, Answer::ArgumentsAsText);
}

#[given(regex = r#"^an MCP server whose `(\S+)` answers the error "(.*)"$"#)]
fn server_answers_error(world: &mut World, mcp_id: String, text: String) {
    let answers = &mut world.mcp.server.answers;
    answers.insert(mcp_id, Answer::Error(text));
}

#[when("the host connects to the MCP server")]
async fn connect(world: &mut World) {
    let (client_io, server_io) = tokio::io::duplex(4096);
    let server = tokio::spawn(rmcp::serve_server(world.mcp.server.clone(), server_io));
    let client = rmcp::serve_client((), client_io)
        .await
        .expect("client connects");
    let server = server.await.expect("server task").expect("server connects");
    match ghostmode::mcp::Server::new(client.peer().clone()).await {
        Ok(mcp_client) => world.host.mcp_client = Some(Arc::new(mcp_client)),
        Err(err) => world.mcp.client_error = Some(err.to_string()),
    }
    world.mcp.services = Some((server, client));
}

#[then(regex = r"^connecting fails with an error naming `(\S+)`$")]
fn connecting_fails(world: &mut World, mcp_id: String) {
    let error = world
        .mcp
        .client_error
        .as_deref()
        .expect("connecting failed");
    assert!(error.contains(&mcp_id), "{error}");
}
