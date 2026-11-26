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
    And the model waited at least 200 ms but under 10 seconds

  Scenario: Dumping while a run is open is an error naming the run
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the caller dumps the sandbox
    Then the caller's error is that run 1 is open

  Scenario: A sandbox restored from a dump made after a reset says the session restarted
    When the model executes x = 1
    And the caller resets the sandbox
    And the caller dumps the sandbox
    And the caller restores the dump into a new sandbox
    And the model executes x
    Then the status is failed with an error naming NameError
    And the status says the session restarted

  Scenario: A restored sandbox numbers its runs on from the dumped sandbox's last run
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes x = 1
    And the caller dumps the sandbox
    And the caller restores the dump into a new sandbox
    And the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 2}

  Scenario Outline: A restore of <bytes> fails with an error and returns the builder, which builds a sandbox over the same host
    Given a host whose `search` answers {"hits": [1, 2]}
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller restores <bytes> into a new sandbox
    Then the caller's error is <error>
    When the caller builds the builder that the failed restore returned
    And the model executes search(query="a")["hits"]
    Then the status is {"status": "finished", "value": [1, 2]}

    Examples:
      | bytes                                 | error                                            |
      | the dump under key "A"                | that the signature doesn't match                 |
      | the bytes "short"                     | that the bytes don't restore                     |
      | the dump with its first byte flipped  | that the dump is format version 0, not version 1 |
      | the dump truncated to half its length | that the bytes don't restore                     |
