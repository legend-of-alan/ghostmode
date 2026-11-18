Feature: A run that outlives its wait keeps going until poll finds it finished

  Scenario: A slow call runs past execute and finishes on poll
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 300 ms and poll wait is 200 ms
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    And the model waited at least 300 ms but under 10 seconds
    When the model executes 1 + 1
    Then the status is failed with an error naming run 1 is still running
    When the model polls run 1
    Then the status is {"status": "running", "run_id": 1}
    And the model waited at least 200 ms but under 10 seconds
    When `search` answers {"hits": []}
    And the model polls run 1
    Then the status is {"status": "finished", "value": {"hits": []}}
    When the model polls run 1
    Then the status is failed with an error naming no run 1

  Scenario: Execute waits 30 seconds and poll waits 10 seconds by default
    Then a default sandbox waits 30 seconds in execute and 10 seconds in poll
