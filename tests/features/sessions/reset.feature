Feature: The caller resets a session

  Scenario: A reset clears the globals and the next status says so
    When the model executes x = 1
    And the caller resets the sandbox
    And the model executes x
    Then the status is failed with an error naming NameError
    And the status says the session restarted

  Scenario: A reset drops the open run and its call
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the caller resets the sandbox
    And the model polls run 1
    Then the status is failed with an error naming no run 1
    And answering `search` finds no run waiting
