@monty-pool
Feature: A worker that crashes fails its run, and the session starts fresh

  Scenario: A crash while a tool call waits fails the run, and the next status says the session restarted
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes x = 1; search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the session's monty worker is killed
    And `search` answers {"hits": []}
    And the model polls run 1
    Then the status is failed with an error naming worker crashed
    When the model executes x
    Then the status is failed with an error naming NameError
    And the status says the session restarted
