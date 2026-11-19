@pool
Feature: A call the caller stops awaiting loses the session, and the next call starts a fresh one

  Scenario: Execute after a dropped execute runs in a fresh session and says it restarted
    When the model executes x = 1
    And the caller stops awaiting the model's execute of x = 2
    And the model executes x
    Then the status is failed with an error naming NameError
    And the status says the session restarted

  Scenario: Dump after a dropped execute writes a fresh session
    When the model executes x = 1
    And the caller stops awaiting the model's execute of x = 2
    And the caller dumps the sandbox
    And the caller restores the dump into a new sandbox
    And the model executes x
    Then the status is failed with an error naming NameError
