Feature: The caller hands its model execute and poll as MCP tools or OpenAI functions

  Scenario: MCP tools and OpenAI functions wrap the same execute and poll
    Given a host whose `search` answers {"hits": []}
    Then the MCP tools are `execute` and `poll`, each with a name, description and inputSchema
    And the OpenAI functions carry the same names, descriptions and schemas

  Scenario: Execute and poll declare exactly their arguments
    Then the `execute` input schema without descriptions is {"type": "object", "properties": {"code": {"type": "string"}, "restart": {"type": "boolean", "default": false}}, "required": ["code"], "additionalProperties": false}
    And the `poll` input schema without descriptions is {"type": "object", "properties": {"run_id": {"type": "integer"}}, "required": ["run_id"], "additionalProperties": false}

