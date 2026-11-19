Feature: A status keeps the head and tail of output over budget, and carries what the script printed to stdout and stderr

  Scenario Outline: <case> over the output budget keeps its head and tail around a marker
    Given a sandbox whose `output_chars` is 10
    When the model executes <script>
    Then the status is <status>

    Examples:
      | case                                        | script                                                               | status                                                                                                                                      |
      | a string                                    | "a" * 1000                                                           | {"status": "finished", "value": "aaaaa[… 990 of 1,000 characters cut …]aaaaa"}                                                                |
      | a list, as its JSON text                    | list(range(100))                                                     | {"status": "finished", "value": "[0,1,[… 281 of 291 characters cut …]8,99]"}                                                                  |
      | an error                                    | raise ValueError("a" * 1000)                                         | {"status": "failed", "error": "Trace[… 1,117 of 1,127 characters cut …]aaaaa", "calls": []}                                                   |
      | a value and its prints                      | print("p" * 1000); "v" * 1000                                        | {"status": "finished", "value": "vvv[… 995 of 1,000 characters cut …]vv", "stdout": "ppp[… 996 of 1,001 characters cut …]p\\n"}               |
      | prints beside a short value                 | print("p" * 1000); 1                                                 | {"status": "finished", "value": 1, "stdout": "ppppp[… 992 of 1,001 characters cut …]ppp\\n"}                                                  |
      | prints on both streams beside a short value | import sys; print("o" * 1000); print("e" * 1000, file=sys.stderr); 1 | {"status": "finished", "value": 1, "stdout": "ooo[… 996 of 1,001 characters cut …]o\\n", "stderr": "ee[… 997 of 1,001 characters cut …]e\\n"} |

  Scenario Outline: A script that <case> returns what it printed
    Given a host whose `search` answers {"hits": []}
    When the model executes <script>
    Then the status is <status>

    Examples:
      | case                        | script                                                     | status                                                                     |
      | prints and then ends        | print("hi"); 1                                             | {"status": "finished", "value": 1, "stdout": "hi\\n"}                      |
      | prints around a tool call   | print("before"); search(query="a"); print("after"); 1      | {"status": "finished", "value": 1, "stdout": "before\\nafter\\n"}          |
      | prints to stdout and stderr | import sys; print("out"); print("err", file=sys.stderr); 1 | {"status": "finished", "value": 1, "stdout": "out\\n", "stderr": "err\\n"} |

  Scenario: A script that prints and then raises fails with its prints in stdout and stderr and the traceback in error
    When the model executes import sys; print("out"); print("hi", file=sys.stderr); raise ValueError("no")
    Then the status is failed with an error naming ValueError
    And the status's stdout is "out\n"
    And the status's stderr is "hi\n"

  Scenario: A call's argument over its share of the output budget is cut when the host answers
    Given a host whose `search` answers {"hits": []}
    And a sandbox whose `output_chars` is 400
    And a sandbox whose `calls_per_execute` is 2
    When the model executes search(query="a" * 1000); raise ValueError("no")
    Then the status is failed with an error naming ValueError
    And the failed status lists the calls [{"tool_id": "search", "args": {"query": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[… 800 of 1,000 characters cut …]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]
