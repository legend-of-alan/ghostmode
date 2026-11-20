//! A host over a connected MCP client, so a script can call the MCP server's tools.

use std::collections::HashMap;
use std::fmt;

use futures_util::future::{self, BoxFuture, FutureExt};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::{Peer, RoleClient, ServiceError};
use serde_json::Value;

use crate::host::{self, Call, Host, Tool};
use crate::schema;

/// A host that wraps the MCP server at the other end of `peer`: it offers the server's tools to
/// scripts and sends each call to the server as `tools/call`. It lists the tools once, when built.
#[derive(Clone, Debug)]
pub struct Server {
    peer: Peer<RoleClient>,
    tools: Vec<Tool>,
    /// Each tool's MCP id, the `name` the server calls it by, by tool id.
    mcp_ids: HashMap<String, String>,
}

impl Server {
    /// Lists the server's tools over `peer`, a connected rmcp client.
    pub async fn new(peer: Peer<RoleClient>) -> Result<Self, Error> {
        let mut tools = Vec::new();
        let mut mcp_ids = HashMap::new();
        for mcp_tool in peer.list_all_tools().await.map_err(Error::ListTools)? {
            let mcp_id = mcp_tool.name.into_owned();
            let tool_id = to_tool_id(&mcp_id);
            if !schema::is_python_identifier(&tool_id) {
                return Err(Error::UncallableId { tool_id });
            }
            if let Some(other) = mcp_ids.insert(tool_id.clone(), mcp_id.clone()) {
                let mcp_ids = [other, mcp_id];
                return Err(Error::SharedId { tool_id, mcp_ids });
            }
            let description = mcp_tool.description.unwrap_or_default();
            let input_schema = Value::Object((*mcp_tool.input_schema).clone());
            tools.push(Tool::new(tool_id, description, input_schema));
        }
        Ok(Self {
            peer,
            tools,
            mcp_ids,
        })
    }
}

impl Host for Server {
    fn tools(&self) -> Vec<Tool> {
        self.tools.clone()
    }

    fn call(&self, call: &Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        let Some(mcp_id) = self.mcp_ids.get(&call.tool_id).cloned() else {
            let message = format!("no MCP tool has the id {}", call.tool_id);
            return future::ready(Err(host::Error::Failed(message))).boxed();
        };
        let peer = self.peer.clone();
        let params = CallToolRequestParams::new(mcp_id).with_arguments(call.args.clone());
        async move {
            let result = peer.call_tool(params).await;
            to_answer(result.map_err(|error| host::Error::Failed(error.to_string()))?)
        }
        .boxed()
    }
}

/// Why `Server` could not offer the server's tools.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Listing the server's tools failed.
    ListTools(ServiceError),
    /// Two MCP tools, these two, map to the same tool id.
    SharedId {
        /// The id both tools map to.
        tool_id: String,
        /// The two tools' MCP ids.
        mcp_ids: [String; 2],
    },
    /// An MCP tool's id isn't a Python identifier, or is a Python keyword or builtin's name, so a
    /// script can't call it.
    UncallableId {
        /// The tool's id, which is also its MCP id.
        tool_id: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListTools(error) => write!(f, "listing the MCP server's tools failed: {error}"),
            Self::SharedId {
                tool_id,
                mcp_ids: [first, second],
            } => write!(
                f,
                "MCP tools `{first}` and `{second}` both map to the id `{tool_id}`"
            ),
            Self::UncallableId { tool_id } => {
                write!(
                    f,
                    "MCP tool `{tool_id}` is not a Python name a script can call"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ListTools(error) => Some(error),
            Self::SharedId { .. } | Self::UncallableId { .. } => None,
        }
    }
}

/// The tool id a script calls an MCP tool by: its MCP id with every character that can't
/// stand there in a Python identifier replaced by `_`.
fn to_tool_id(mcp_id: &str) -> String {
    let mut tool_id: String = mcp_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if tool_id.starts_with(|c: char| c.is_ascii_digit()) {
        tool_id.replace_range(..1, "_");
    }
    tool_id
}

/// The host's answer from an MCP `tools/call` result: its structured content when present, else
/// its text, parsed as JSON when it parses. `isError: true` fails the call with the text.
fn to_answer(result: CallToolResult) -> Result<Value, host::Error> {
    let text: Vec<&str> = result
        .content
        .iter()
        .filter_map(|content| content.as_text())
        .map(|content| content.text.as_str())
        .collect();
    let text = text.join("\n");
    if result.is_error == Some(true) {
        return Err(host::Error::Failed(text));
    }
    if let Some(value) = result.structured_content {
        return Ok(value);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}
