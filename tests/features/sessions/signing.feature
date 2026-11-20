Feature: With a signing key, dump signs the session and restore refuses bytes the key did not sign

  Scenario: A dump restores under the key that signed it
    Given a sandbox whose signing key is "A"
    When the model executes x = 7
    And the caller dumps the sandbox
    And the model executes x = 8
    And the caller restores the dump into a new sandbox
    And the model executes x
    Then the status is {"status": "finished", "value": 7}

  Scenario Outline: Restore under key "A" refuses <bytes>
    Given a sandbox whose signing key is "A"
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller restores <bytes> into a new sandbox
    Then the caller's error is that the signature doesn't match

    Examples:
      | bytes                                 |
      | the dump with its first byte flipped  |
      | the dump with its last byte flipped   |
      | the bytes "short"                     |

  Scenario: Restore refuses a dump that another key signed
    Given a sandbox whose signing key is "A"
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller restores the dump under key "B" into a new sandbox
    Then the caller's error is that the signature doesn't match

  Scenario: Restore under a key refuses a dump made without one
    When the model executes x = 7
    And the caller dumps the sandbox
    And the caller restores the dump under key "A" into a new sandbox
    Then the caller's error is that the signature doesn't match
