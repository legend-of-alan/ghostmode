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
