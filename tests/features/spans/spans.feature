Feature: Spans carry ids, statuses, durations and counts, never a script's data

  Scenario: Execute and call spans hold only ids, statuses, durations and counts, and a call span has no parent
    Given a host whose `search` answers {"hits": ["secret-hit"]}
    And a subscriber that captures every span
    When the model executes print("secret-print"); search(query="secret-query")["hits"]
    Then the status is {"status": "finished", "value": ["secret-hit"], "stdout": "secret-print\n"}
    And the captured spans are execute and call
    And the captured span fields are only run_id, status, duration, tool_id and call_count
    And the call span has no parent
    And the call span's `status` is "answered"
    And the call span's `run_id` equals the execute span's
    And no captured span field holds "secret"

  Scenario: Cancelling a run closes its waiting call span as dropped, and the cancel span carries the run id
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 10 ms and poll wait is 10 ms
    And a subscriber that captures every span
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the caller cancels run 1
    Then the call span's `status` is "dropped"
    And the call span records a `duration`
    And the cancel span's `run_id` is 1
    And the cancel span's `status` is "cancelled"

  Scenario: An execute that expires an idle run records its run id, and the waiting call span closes as dropped
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 10 ms and poll wait is 10 ms
    And a sandbox whose `idle_timeout` is 50 ms
    And a subscriber that captures every span
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When 60 ms pass
    And the model executes 1
    Then the status is failed with an error naming run 1 expired
    And the call span's `status` is "dropped"
    And the call span records a `duration`
    And the execute span's `run_id` is 1
