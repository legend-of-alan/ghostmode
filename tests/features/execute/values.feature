Feature: Python values come back as the JSON the model reads

  Scenario Outline: <script> comes back as <json>
    When the model executes <script>
    Then the status is {"status": "finished", "value": <json>}

    Examples:
      | script   | json                     |
      | None     | null                     |
      | (1, 2)   | [1, 2]                   |
      | {3}      | [3]                      |
      | {1: 'a'} | {"1": "a"}               |
      | 2 ** 70  | "1180591620717411303424" |
      | range(3) | "range(0, 3)"            |

  Scenario: A value nested more than 126 levels fails, and the next script still runs
    When the model executes this script:
      """
      x = 1
      for i in range(127):
          x = [x]
      x
      """
    Then the status is failed with an error naming nested more than 126 levels
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}

  Scenario: A value 999 levels deep fails, and the next script still runs
    When the model executes this script:
      """
      x = 1
      for i in range(999):
          x = [x]
      x
      """
    Then the status is failed with an error naming nested more than 126 levels
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}

  Scenario: A tool argument nested more than 126 levels raises in the script, and the next script still runs
    Given a host whose `search` answers {"hits": []}
    When the model executes this script:
      """
      y = 1
      for i in range(127):
          y = [y]
      search(query=y)
      """
    Then the status is failed with an error naming ValueError
    And the status is failed with an error naming nested more than 126 levels
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}

  Scenario: A host answer nested more than 126 levels raises in the script, and the next script still runs
    Given a host whose `search` answers a list nested 100000 levels
    When the model executes search(query="a")
    Then the status is failed with an error naming ValueError
    And the status is failed with an error naming nested more than 126 levels
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}

  Scenario: A value that holds one list 2 ** 23 times fails, and the next script still runs
    When the model executes this script:
      """
      a = [0]
      for i in range(23):
          a = [a, a]
      a
      """
    Then the status is failed with an error naming more than 10485760 bytes as JSON
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}

  Scenario Outline: <script> fails because it takes more than 10 MiB as JSON
    When the model executes <script>
    Then the status is failed with an error naming more than 10485760 bytes as JSON

    Examples:
      | script                          |
      | s = 'x' * 5000000; [s] * 100    |
      | 1 << 40000000                   |
      | 'x' * 10485761                  |

  Scenario: A tool argument that takes more than 10 MiB as JSON raises in the script
    Given a host whose `search` answers {"hits": []}
    When the model executes search(query=['x' * 5000000] * 3)
    Then the status is failed with an error naming ValueError
    And the status is failed with an error naming more than 10485760 bytes as JSON

  Scenario: A dict whose keys 1 and '1' both become the JSON key "1" fails
    When the model executes {1: 'a', '1': 'b'}
    Then the status is failed with an error naming the JSON key "1" twice

  Scenario: A tool argument Monty cuts short for nesting raises in the script
    Given a sandbox whose `recursion_depth` is 20
    And a host whose `search` answers {"hits": []}
    When the model executes this script:
      """
      y = 1
      for i in range(30):
          y = [y]
      search(query=y)
      """
    Then the status is failed with an error naming ValueError
    And the status is failed with an error naming nested more than 126 levels

  Scenario: A host answer above i64::MAX reaches the script as that exact int
    Given a host whose `search` answers {"n": 10000000000000000000}
    When the model executes repr(search(query="a")["n"])
    Then the status is {"status": "finished", "value": "10000000000000000000"}
