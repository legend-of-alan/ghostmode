Feature: A status keeps the head and tail of output over budget, and carries what the script printed to stdout and stderr

  Scenario Outline: <case> over the output budget keeps its head and tail around a marker, all within the budget
    Given a sandbox whose `output_chars` is <budget>
    When the model executes <script>
    Then the status is <status>

    Examples:
      | case                                        | budget | script                                                               | status                                                                                                                                                            |
      | a string                                    | 50     | "a" * 1000                                                           | {"status": "finished", "value": "aaaaa[… 991 of 1,000 characters truncated …]aaaa"}                                                                               |
      | a string, on a budget smaller than a marker | 10     | "a" * 1000                                                           | {"status": "finished", "value": "[… 1,000 of 1,000 characters truncated …]"}                                                                                            |
      | a list, as its JSON text                    | 50     | list(range(100))                                                     | {"status": "finished", "value": "[0,1,2,[… 278 of 291 characters truncated …]98,99]"}                                                                             |
      | an error                                    | 50     | raise ValueError("a" * 1000)                                         | {"status": "failed", "error": "Trace[… 1,118 of 1,127 characters truncated …]aaaa", "calls": []}                                                                  |
      | a value and its prints                      | 100    | print("p" * 1000); "v" * 1000                                        | {"status": "finished", "value": "vvvvv[… 991 of 1,000 characters truncated …]vvvv", "stdout": "ppppp[… 992 of 1,001 characters truncated …]ppp\\n"}               |
      | prints beside a short value                 | 50     | print("p" * 1000); 1                                                 | {"status": "finished", "value": 1, "stdout": "pppp[… 993 of 1,001 characters truncated …]ppp\\n"}                                                                 |
      | prints on both streams beside a short value | 100    | import sys; print("o" * 1000); print("e" * 1000, file=sys.stderr); 1 | {"status": "finished", "value": 1, "stdout": "ooooo[… 992 of 1,001 characters truncated …]ooo\\n", "stderr": "eeee[… 993 of 1,001 characters truncated …]eee\\n"} |

  Scenario Outline: A script that <case> returns what it printed
    Given a host whose `search` answers {"hits": []}
    When the model executes <script>
    Then the status is <status>

    Examples:
      | case                      | script                                                | status                                                            |
      | prints around a tool call | print("before"); search(query="a"); print("after"); 1 | {"status": "finished", "value": 1, "stdout": "before\\nafter\\n"} |

  Scenario: A script that prints and then raises fails with its prints in stdout and stderr and the traceback in error
    When the model executes import sys; print("out"); print("hi", file=sys.stderr); raise ValueError("no")
    Then the status is failed with an error naming ValueError
    And the status's stdout is "out\n"
    And the status's stderr is "hi\n"

  Scenario Outline: A failed status's calls keep their arguments, names included, within each call's share of the output budget, for <case>
    Given a host whose `search` answers {"hits": []}
    And a sandbox whose `output_chars` is 400
    And a sandbox whose `calls_per_execute` is <calls_per_execute>
    When the model executes <script>; raise ValueError("no")
    Then the status is failed with an error naming ValueError
    And the failed status lists the calls <calls>

    Examples:
      | case                              | calls_per_execute | script                                             | calls                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
      | one long argument                 | 2                 | search(query="a" * 1000)                           | [{"tool_id": "search", "args": {"query": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[… 846 of 1,000 characters truncated …]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]                                                                                                                                                                                                                                                |
      | many arguments                    | 2                 | search(a="x" * 150, b="y" * 150, c="z" * 150)      | [{"tool_id": "search", "args": {"a": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}, "truncated_args": "{\\"b\\":\\"[… 303 of 315 characters truncated …]zzzz\\"}"}] |
      | an argument name over the share   | 2                 | search(**{"k" * 250: "v"})                         | [{"tool_id": "search", "args": {}, "truncated_args": "{\\"kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk[… 95 of 258 characters truncated …]kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk\\":\\"v\\"}"}] |
      | a share smaller than the marker   | 20                | search(**{"k" * 250: "v"})                         | [{"tool_id": "search", "args": {}, "truncated_args": "[… 258 of 258 characters truncated …]"}] |
      | as many calls as the limit allows | 2                 | search(query="a" * 1000); search(query="b" * 1000) | [{"tool_id": "search", "args": {"query": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[… 846 of 1,000 characters truncated …]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}, {"tool_id": "search", "args": {"query": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb[… 846 of 1,000 characters truncated …]bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}] |
