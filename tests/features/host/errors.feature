Feature: A host's error is a std error

  Scenario Outline: The caller boxes a host's <answer> as a std error that reads as its text
    Given a host that refuses `write` with "read-only workspace"
    And a host whose `fetch` fails with "connection reset"
    When the caller calls `<tool_id>` on the host
    Then the host's error reads "<text>"

    Examples:
      | answer  | tool_id | text                |
      | refusal | write   | read-only workspace |
      | failure | fetch   | connection reset    |
