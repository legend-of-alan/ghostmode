Feature: The caller hands its model execute and poll as MCP tools or OpenAI functions

  Scenario: MCP tools and OpenAI functions wrap the same execute and poll
    Given a host whose `search` answers {"hits": []}
    Then the MCP tools are `execute` and `poll`, each with a name, description and inputSchema
    And the OpenAI functions carry the same names, descriptions and schemas

  Scenario: Execute and poll declare exactly the arguments their calls take
    When the model calls `execute` with {"code": "1 + 1"}
    Then the status is {"status": "finished", "value": 2}
    When the model calls `poll` with {"run_id": 1}
    Then the status is failed with an error naming no run 1
    And the `execute` schema declares exactly `code` as a string, required, with additionalProperties false
    And the `poll` schema declares exactly `run_id` as an integer, required, with additionalProperties false

  Scenario: The execute description lists each tool as a keyword-only signature
    Given a host whose `search` input schema is {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}, "after": {"type": "string"}}, "required": ["query"]}
    And a host whose `ping` answers {}
    Then the execute description contains "search(*, query, after, limit)"
    And the execute description contains "ping()"
    And the execute description contains "keyword arguments only"
