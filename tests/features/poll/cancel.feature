Feature: The caller cancels a run that execute left running

  @monty
  Scenario: Cancelling the open run keeps its globals and drops its call, and an unknown run changes nothing
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes z = 7; search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the caller cancels run 9
    Then cancel returns false
    When the caller cancels run 1
    Then cancel returns true
    When the model polls run 1
    Then the status is failed with an error naming no run 1
    When the model executes z
    Then the status is {"status": "finished", "value": 7}
    And answering `search` finds no run waiting

  @pool
  Scenario: Cancelling the open run ends its worker with the session's globals and drops its call
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes z = 7; search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the caller cancels run 1
    Then cancel returns true
    When the model executes z
    Then the status is failed with an error naming NameError
    And the status says the session restarted
    And answering `search` finds no run waiting
