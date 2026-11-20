@rmcp
Feature: An MCP client offers its server's tools to scripts

  Scenario: A tool's structured content comes back as the script's value
    Given an MCP server whose `get_weather` answers its arguments as structured content
    When the host connects to the MCP server
    And the model executes get_weather(city="Oslo")
    Then the status is {"status": "finished", "value": {"city": "Oslo"}}

  Scenario: A tool whose name is no Python identifier is called by its id, and its JSON text is parsed
    Given an MCP server whose `1get-weather.v2` answers its arguments as text
    When the host connects to the MCP server
    And the model executes _get_weather_v2(city="Oslo")["city"]
    Then the status is {"status": "finished", "value": "Oslo"}

  Scenario: A tool's text that isn't JSON comes back as a string
    Given an MCP server whose `forecast` answers the text "sunny"
    When the host connects to the MCP server
    And the model executes forecast()
    Then the status is {"status": "finished", "value": "sunny"}

  Scenario: A tool's error fails the script with the server's text
    Given an MCP server whose `broken` answers the error "no forecast today"
    When the host connects to the MCP server
    And the model executes broken()
    Then the status is failed with an error naming RuntimeError
    And the status is failed with an error naming no forecast today

  Scenario: Connecting fails naming both tools when two tools map to one id
    Given an MCP server whose `get.weather` answers its arguments as text
    And the MCP server's `get-weather` also answers its arguments as text
    When the host connects to the MCP server
    Then connecting fails with an error naming `get.weather`
    And connecting fails with an error naming `get-weather`

  Scenario Outline: Connecting fails naming a tool whose id a script can't call
    Given an MCP server whose `<mcp_id>` answers its arguments as text
    When the host connects to the MCP server
    Then connecting fails with the error "MCP tool `<mcp_id>` is not a Python name a script can call"

    Examples:
      | mcp_id |
      | class  |
      |        |
