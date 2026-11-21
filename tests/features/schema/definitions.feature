Feature: The caller hands its model the execute and poll definitions in MCP or OpenAI shape

  Scenario: The MCP and OpenAI definitions wrap the same execute and poll
    Given a host whose `search` answers {"hits": []}
    Then the MCP definitions are `execute` and `poll`, each with a name, description, inputSchema and annotations
    And the MCP definitions' annotations are {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}
    And the OpenAI definitions carry the same names, descriptions and schemas

  @rmcp
  Scenario: An MCP client reads the MCP definitions as its own tools
    Given a host whose `search` answers {"hits": []}
    Then rmcp reads the MCP definitions as the tools `execute` and `poll` with their schemas

  Scenario: Execute and poll declare exactly their arguments
    Then the `execute` input schema without descriptions is {"type": "object", "properties": {"code": {"type": "string"}, "restart": {"type": "boolean", "default": false}}, "required": ["code"], "additionalProperties": false}
    And the `poll` input schema without descriptions is {"type": "object", "properties": {"run_id": {"type": "integer"}}, "required": ["run_id"], "additionalProperties": false}

