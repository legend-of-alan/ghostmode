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

  Scenario: A run that starts in the fresh worker after a crash and expires says the session restarted
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    And a sandbox whose `idle_timeout` is 50 ms
    When the model executes x = 1
    And the session's monty worker is killed
    And the model executes x = 2
    Then the status is failed with an error naming worker crashed
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 3}
    When 60 ms pass
    And the model polls run 3
    Then the status is failed with an error naming run 3 expired
    And the status says the session restarted
