Feature: The caller hands its model execute and poll as MCP tools or OpenAI functions

  Scenario: MCP tools and OpenAI functions wrap the same execute and poll
    Given a host whose `search` answers {"hits": []}
    Then the MCP tools are `execute` and `poll`, each with a name, description and inputSchema
    And the OpenAI functions carry the same names, descriptions and schemas

  Scenario: Execute and poll declare exactly the arguments their calls take
    When the model calls `execute` with {"code": "1 + 1", "restart": true}
    Then the status is {"status": "finished", "value": 2, "restarted": true}
    When the model calls `poll` with {"run_id": 1}
    Then the status is failed with an error naming no run 1
    And the `execute` schema requires exactly `code` as a string, with additionalProperties false
    And the `execute` schema declares `restart` as a boolean, not required, defaulting to false
    And the `poll` schema requires exactly `run_id` as an integer, with additionalProperties false

