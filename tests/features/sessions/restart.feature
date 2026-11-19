Feature: Execute's restart flag starts the session fresh

  Scenario: A restart clears the globals and the next status says so once
    When the model executes x = 1
    And the model executes x with restart
    Then the status is failed with an error naming NameError
    And the status says the session restarted
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}
