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

  Scenario: A value nested more than 127 levels fails, and the next script still runs
    When the model executes this script:
      """
      x = 1
      for i in range(128):
          x = [x]
      x
      """
    Then the status is failed with an error naming nested more than 127 levels
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}
