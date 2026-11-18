Feature: A failed script names its cause

  Scenario Outline: <case> ends failed with <error>
    Given a host whose `search` answers {"hits": [1, 2]}
    And a host that refuses `write` with "read-only workspace"
    And a host whose `fetch` fails with "connection reset"
    When the model executes <script>
    Then the status is failed with an error naming <error>
    And the failed status lists the calls <calls>

    Examples:
      | case                  | script                                    | error           | calls                                                                                                  |
      | an exception          | search(query="a"); raise ValueError("no") | ValueError      | [{"tool_id": "search", "args": {"query": "a"}}]                                                        |
      | a syntax error        | def (                                     | SyntaxError     | []                                                                                                     |
      | an uncaught refusal   | search(query="a"); write(path="a")        | PermissionError | [{"tool_id": "search", "args": {"query": "a"}}, {"tool_id": "write", "args": {"path": "a"}}]           |
      | a failed call         | fetch(url="u")                            | RuntimeError    | [{"tool_id": "fetch", "args": {"url": "u"}}]                                                           |
      | opening a file        | open("x.txt")                             | PermissionError | []                                                                                                     |
      | a positional argument | search("a")                               | TypeError       | []                                                                                                     |
      | an unknown tool       | delete(path="a")                          | NameError       | []                                                                                                     |
