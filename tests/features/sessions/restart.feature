Feature: Execute's restart flag starts the session fresh

  Scenario: A restart clears the globals and the next status says so once
    When the model executes x = 1
    And the model executes x with restart
    Then the status is failed with an error naming NameError
    And the status says the session restarted
    When the model executes 1 + 1
    Then the status is {"status": "finished", "value": 2}

  Scenario: A restart while a run is open drops that run and runs the script in a fresh session
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 100 ms and poll wait is 100 ms
    When the model executes x = 1; search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the model executes x with restart
    Then the status is failed with an error naming NameError
    And the status says the session restarted
    When the model polls run 1
    Then the status is failed with an error naming no run 1
