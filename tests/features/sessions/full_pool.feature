@monty-pool
Feature: A new session on a full pool fails until another sandbox drops

  Scenario: Execute on a full pool fails saying no worker is free, and runs once another sandbox drops
    When every monty worker is held by another sandbox
    And the model executes x = 1
    Then the status is failed with an error naming no monty worker became available
    When one of the other sandboxes drops
    And the model executes x = 1; x
    Then the status is {"status": "finished", "value": 1}

  Scenario: Restore on a full pool fails, and restores once another sandbox drops
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller drops the sandbox
    And every monty worker is held by another sandbox
    And the caller restores the dump into a new sandbox
    Then the caller's error is that the bytes don't restore
    When one of the other sandboxes drops
    And the caller restores the dump into a new sandbox
    And the model executes x
    Then the status is {"status": "finished", "value": 7}
