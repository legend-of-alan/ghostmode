Feature: A run that outlives its wait keeps going until poll finds it finished

  Scenario: With the default waits, a slow call runs past execute and finishes on poll
    Given a host whose `search` answers only when told to
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    And the model waited at least 30 seconds
    When the model executes 1 + 1
    Then the status is failed with an error naming run 1 is still running
    When the model polls run 1
    Then the status is {"status": "running", "run_id": 1}
    And the model waited at least 10 seconds
    When `search` answers {"hits": []}
    And the model polls run 1
    Then the status is {"status": "finished", "value": {"hits": []}}
    When the model polls run 1
    Then the status is failed with an error naming no run 1

  Scenario: The builder sets both waits
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 300 ms and poll wait is 200 ms
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    And the model waited at least 300 ms but under 10 seconds
    When the model polls run 1
    Then the status is {"status": "running", "run_id": 1}
    And the model waited at least 200 ms but under 10 seconds
