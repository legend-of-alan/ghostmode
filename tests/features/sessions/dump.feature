Feature: The caller dumps a session as bytes and restores it into a new sandbox

  Scenario: A restored sandbox has the globals the dump had
    When the model executes x = 7
    And the caller dumps the sandbox
    And the model executes x = 8
    And the caller restores the dump into a new sandbox
    And the model executes x
    Then the status is {"status": "finished", "value": 7}

  Scenario: A restored sandbox runs under the new builder's limits
    When the model executes x = 1
    And the caller dumps the sandbox
    Given a sandbox whose `run_time` is 200 ms
    When the caller restores the dump into a new sandbox
    And the model executes while True: pass
    Then the status is failed with an error naming time limit exceeded

  Scenario: Dumping while a run is open is an error naming the run
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the caller dumps the sandbox
    Then the caller's error is that run 1 is open

  Scenario: Restoring bytes that are not a dump is Monty's NotADump
    When the caller restores the bytes "not a dump" into a new sandbox
    Then the caller's error is Monty's NotADump
