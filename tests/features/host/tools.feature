Feature: A sandbox keeps the tools its host listed when it was built

  Scenario: A tool the host lists after the build is not offered or callable, and `search` still answers
    Given a host whose `search` answers {"hits": [1, 2]}
    When the model executes search(query="a")["hits"]
    And the host begins to list `delete`, which answers {"deleted": true}
    Then the execute description doesn't contain "def delete("
    When the model executes delete(path="a")
    Then the status is failed with an error naming NameError
    When the model executes search(query="a")["hits"]
    Then the status is {"status": "finished", "value": [1, 2]}
