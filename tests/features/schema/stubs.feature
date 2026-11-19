Feature: The execute description lists each host tool as a typed Python stub

  Scenario: Each tool is a keyword-only stub with its description as the docstring
    Given a host whose `search` input schema is {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}, "after": {"type": "string"}}, "required": ["query"]}
    And a host whose `search` description is "Search issues."
    And a host whose `ping` answers {}
    Then the execute description contains:
      ```
      def search(*, query: str, after: str | None = None, limit: int | None = None) -> Any:
          """Search issues."""
      ```
    And the execute description contains "def ping() -> Any:"
    And the execute description contains "keyword arguments only"

  Scenario Outline: Each argument's JSON Schema becomes a Python type
    Given a host whose `pick` input schema is {"type": "object", "properties": {"choice": <schema>}, "required": ["choice"]}
    Then the execute description contains "def pick(*, choice: <type>) -> Any:"

    Examples:
      | schema                                          | type              |
      | {"type": "string"}                              | str               |
      | {"type": "integer"}                             | int               |
      | {"type": "number"}                              | float             |
      | {"type": "boolean"}                             | bool              |
      | {"type": "array", "items": {"type": "integer"}} | list[int]         |
      | {"type": "object"}                              | dict[str, Any]    |
      | {"type": "string", "enum": ["a", "b"]}          | Literal["a", "b"] |
      | {"description": "No type."}                     | Any               |

  Scenario: A description's backslashes and quotes are escaped in the docstring
    Given a host whose `find` description is "Finds a\b or """x""""
    Then the execute description contains """"Finds a\\b or \"\"\"x\"\"\"""""

  Scenario Outline: A tool whose id isn't a Python identifier gets no stub, and `is_python_identifier` says so
    Given a host whose `<id>` answers {}
    And a host whose `ping` answers {}
    Then the execute description contains "def ping() -> Any:"
    And the execute description doesn't contain "def <id>("
    And `ping` is a Python identifier
    And `<id>` is not a Python identifier

    Examples:
      | id       |
      | get-room |
      | class    |
