Feature: Spans carry ids, statuses, durations and counts, never a script's data

  Scenario: A script's source, arguments and result never reach a span
    Given a host whose `search` answers {"hits": ["secret-hit"]}
    And a subscriber that captures every span
    When the model executes search(query="secret-query")["hits"]
    Then the status is {"status": "finished", "value": ["secret-hit"]}
    And the captured spans are execute and call
    And the captured span fields are only run_id, status, duration, tool_id and call_count
    And the call span has no parent
    And no captured span field holds "secret"
