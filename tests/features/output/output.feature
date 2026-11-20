Feature: A status keeps the head and tail of output over budget, and carries what the script printed to stdout and stderr

  Scenario Outline: <case> over the output budget keeps its head and tail around a marker, all within the budget
    Given a sandbox whose `output_chars` is <budget>
    When the model executes <script>
    Then the status is <status>

    Examples:
      | case                                        | budget | script                                                               | status |
      | a string | 50 | "a" * 1000 | {"status": "finished", "value": "aaaaaaaa[… 985 of 1,000 characters cut …]aaaaaaa"} |
      | a string, on a budget smaller than a marker | 10 | "a" * 1000 | {"status": "finished", "value": "[… 1,000 of 1,000 characters cut …]"} |
      | a list, as its JSON text | 50 | list(range(100)) | {"status": "finished", "value": "[0,1,2,3,4[… 272 of 291 characters cut …]97,98,99]"} |
      | an error | 50 | raise ValueError("a" * 1000) | {"status": "failed", "error": "Tracebac[… 1,112 of 1,127 characters cut …]aaaaaaa", "calls": []} |
      | a value and its prints | 100 | print("p" * 1000); "v" * 1000 | {"status": "finished", "value": "vvvvvvvv[… 985 of 1,000 characters cut …]vvvvvvv", "stdout": "pppppppp[… 986 of 1,001 characters cut …]pppppp\\n"} |
      | prints beside a short value | 50 | print("p" * 1000); 1 | {"status": "finished", "value": 1, "stdout": "ppppppp[… 987 of 1,001 characters cut …]pppppp\\n"} |
      | prints on both streams beside a short value | 100 | import sys; print("o" * 1000); print("e" * 1000, file=sys.stderr); 1 | {"status": "finished", "value": 1, "stdout": "oooooooo[… 986 of 1,001 characters cut …]oooooo\\n", "stderr": "eeeeeee[… 987 of 1,001 characters cut …]eeeeee\\n"} |

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
    And a sandbox whose `calls_per_execute` is 2
    When the model executes <script>; raise ValueError("no")
    Then the status is failed with an error naming ValueError
    And the failed status lists the calls <calls>

    Examples:
      | case                             | script                                                    | calls |
      | one long argument | search(query="a" * 1000) | [{"tool_id": "search", "args": {"query": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[… 840 of 1,000 characters cut …]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}] |
      | many arguments | search(a="x" * 150, b="y" * 150, c="z" * 150) | [{"tool_id": "search", "args": {"a": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "b": "yyyyyyyyy[… 133 of 150 characters cut …]yyyyyyyy"}}] |
      | an argument name over the share  | search(**{"k" * 250: "v"})                                | [{"tool_id": "search", "args": {}}] |
      | as many calls as the limit allows | search(query="a" * 1000); search(query="b" * 1000) | [{"tool_id": "search", "args": {"query": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[… 840 of 1,000 characters cut …]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}, {"tool_id": "search", "args": {"query": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb[… 840 of 1,000 characters cut …]bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}] |
