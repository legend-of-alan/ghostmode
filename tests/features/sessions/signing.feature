Feature: With a signing key, dump signs the session and restore refuses bytes the key did not sign

  Scenario: A dump restores under the key that signed it
    Given a sandbox whose signing key is "A"
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller restores the dump into a new sandbox
    And the model executes x
    Then the status is {"status": "finished", "value": 7}

  Scenario Outline: Restore refuses a signed dump <change>
    Given a sandbox whose signing key is "A"
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller restores the dump <change> into a new sandbox
    Then the caller's error is that the signature doesn't match

    Examples:
      | change                |
      | under key "B"         |
      | without its tag       |
      | with one byte flipped |
