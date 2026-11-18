Feature: The caller hands its model execute and poll as MCP tools or OpenAI functions

  Scenario: MCP tools and OpenAI functions wrap the same execute and poll
    Given a host whose `search` answers {"hits": []}
    Then the MCP tools are `execute` and `poll`, each with a name, description and inputSchema
    And the OpenAI functions carry the same names, descriptions and schemas

  Scenario: Calls built from the schemas reach execute and poll
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 5000 ms
    When the model calls `execute` with {"code": "search(query=\"a\")[\"hits\"]"}
    Then the status is {"status": "running", "run_id": 1}
    When `search` answers {"hits": [1, 2]}
    And the model calls `poll` with {"run_id": 1}
    Then the status is {"status": "finished", "value": [1, 2]}

  Scenario: The execute description lists each tool as a keyword-only signature
    Given a host whose `search` input schema is {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}, "after": {"type": "string"}}, "required": ["query"]}
    And a host whose `ping` answers {}
    Then the execute description contains "search(*, query, after, limit)"
    And the execute description contains "ping()"
    And the execute description contains "keyword arguments only"
