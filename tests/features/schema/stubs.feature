Feature: The execute description lists each host tool as a typed Python stub

  Scenario: Each tool is a keyword-only stub with its description as the docstring, even with an argument named like a builtin
    Given a host whose `search` input schema is {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}, "after": {"type": "string"}, "id": {"type": "string"}}, "required": ["query"]}
    And a host whose `search` description is "Search issues."
    And a host whose `ping` answers {}
    Then the execute description contains:
      ```
      def search(*, query: str, after: str | None = None, id: str | None = None, limit: int | None = None) -> Any:
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

  Scenario: Each line of a multi-line description is indented in the docstring
    Given a host whose `search` description is:
      """
      Search issues.
      Closed issues come last.
      """
    Then the execute description contains:
      ```
      def search() -> Any:
          """Search issues.
          Closed issues come last."""
      ```

  Scenario: A description's backslashes and quotes are escaped in the docstring
    Given a host whose `find` description is "Finds a\b or """x""""
    Then the execute description contains """"Finds a\\b or \"\"\"x\"\"\"""""

  Scenario Outline: A tool with an argument that can't stand in a def takes **kwargs, listed in its docstring
    Given a host whose `lookup` input schema is {"type": "object", "properties": {"<arg>": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["<arg>"]}
    And a host whose `lookup` description is "Looks up a user."
    Then the execute description contains:
      ```
      def lookup(**kwargs: Any) -> Any:
          """Looks up a user.
          Keyword arguments:
          <arg>: str
          limit: int | None = None"""
      ```
    When the model executes lookup(**{"<arg>": "42"}); raise ValueError("no")
    Then the failed status lists the calls [{"tool_id": "lookup", "args": {"<arg>": "42"}}]

    Examples:
      | arg       |
      | user-id   |
      | __debug__ |

  Scenario Outline: A tool whose id a script can't call gets no stub
    Given a host whose `<id>` answers {}
    And a host whose `ping` answers {}
    Then the execute description contains "def ping() -> Any:"
    And the execute description doesn't contain "def <id>("

    Examples:
      | id                  |
      | get-room            |
      | class               |
      | 1st                 |
      | print               |
      | len                 |
      | open                |
      | FrozenInstanceError |
