Feature: Execute runs a script that calls the host's tools

  Scenario: A tool call's answer comes back as the script's value
    Given a host whose `search` answers {"hits": [1, 2]}
    When the model executes search(query="a")["hits"]
    Then the status is {"status": "finished", "value": [1, 2]}

  Scenario: Variables persist from one execute to the next
    When the model executes x = 41
    And the model executes x + 1
    Then the status is {"status": "finished", "value": 42}

  Scenario: A script catches a refusal and reads its reason
    Given a host that refuses `write` with "read-only workspace"
    When the model executes this script:
      """
      try:
          write(path="a")
      except PermissionError as error:
          reason = str(error)
      reason
      """
    Then the status is {"status": "finished", "value": "read-only workspace"}
