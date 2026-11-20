Feature: Limits stop a script, and the session keeps its variables

  Scenario: A script that never calls a tool stops at the run time limit, and the session runs on
    Given a sandbox whose `run_time` is 200 ms
    When the model executes x = 1
    And the model executes while True: pass
    Then the status is failed with an error naming time limit exceeded
    And the model waited at least 200 ms but under 10 seconds
    When the model executes x
    Then the status is {"status": "finished", "value": 1}

  Scenario: Unbounded recursion stops at the recursion depth
    When the model executes def f(n): return f(n + 1); f(0)
    Then the status is failed with an error naming RecursionError

  Scenario: One allocation over the memory limit fails
    Given a sandbox whose `memory_bytes` is 64 MiB
    When the model executes "a" * (128 * 1024 * 1024)
    Then the status is failed with an error naming MemoryError

  @monty-pool
  Scenario: A worker stops a script that grows past the memory limit a megabyte at a time
    Given a sandbox whose `memory_bytes` is 64 MiB
    When the model executes this script:
      """
      chunks = []
      for i in range(128):
          chunks.append("a" * (1024 * 1024))
      """
    Then the status is failed with an error naming MemoryError

  Scenario: The call limit stops a script even inside a try that catches everything
    Given a host whose `search` answers {"hits": []}
    And a sandbox whose `calls_per_execute` is 3
    When the model executes this script:
      """
      try:
          for i in range(5):
              search(query="a")
      except BaseException:
          pass
      """
    Then the status is failed with an error naming call limit
    And the failed status lists the calls [{"tool_id": "search", "args": {"query": "a"}}, {"tool_id": "search", "args": {"query": "a"}}, {"tool_id": "search", "args": {"query": "a"}}]

  @monty
  Scenario: A call the host holds past the call timeout fails, and the session runs on
    Given a host whose `search` answers only when told to
    And a sandbox whose `call_timeout` is 50 ms
    When the model executes x = 1
    And the model executes search(query="a")
    Then the status is failed with an error naming call timeout
    When the model executes x
    Then the status is {"status": "finished", "value": 1}

  @monty-pool
  Scenario: A call the host holds past the call timeout fails, and the worker ends with the session's globals
    Given a host whose `search` answers only when told to
    And a sandbox whose `call_timeout` is 50 ms
    When the model executes x = 1
    And the model executes search(query="a")
    Then the status is failed with an error naming call timeout
    When the model executes x
    Then the status is failed with an error naming NameError
    And the status says the session restarted

  Scenario: A run polled inside the idle timeout stays running, then finishes
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 10 ms and poll wait is 10 ms
    And a sandbox whose `idle_timeout` is 10000 ms
    When the model executes search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When the model polls run 1
    Then the status is {"status": "running", "run_id": 1}
    When `search` answers {"hits": []}
    And the model polls run 1
    Then the status is {"status": "finished", "value": {"hits": []}}

  @monty
  Scenario: A run left unpolled past the idle timeout expires, and the session runs on
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 10 ms and poll wait is 10 ms
    And a sandbox whose `idle_timeout` is 50 ms
    When the model executes x = 1; search(query="a")
    Then the status is {"status": "running", "run_id": 1}
    When 60 ms pass
    And the model polls run 1
    Then the status is failed with an error naming run 1 expired
    When the model executes x
    Then the status is {"status": "finished", "value": 1}

  Scenario: A run whose host answered stays open past the idle timeout, and the next poll finishes it
    Given a host whose `search` answers only when told to
    And a sandbox whose execute wait is 10 ms and poll wait is 10 ms
    And a sandbox whose `idle_timeout` is 50 ms
    When the model executes hits = search(query="a"); hits
    Then the status is {"status": "running", "run_id": 1}
    When `search` answers {"hits": []}
    And 60 ms pass
    And the model executes hits
    Then the status is failed with an error naming run 1 is still running
    When the model polls run 1
    Then the status is {"status": "finished", "value": {"hits": []}}
    When the model executes hits
    Then the status is {"status": "finished", "value": {"hits": []}}

  @monty
  Scenario: An execute after the idle timeout answers the expired run's calls instead of running
    Given a host whose `search` answers {"hits": []}
    And a host whose `fetch` answers only when told to
    And a sandbox whose execute wait is 10 ms and poll wait is 10 ms
    And a sandbox whose `idle_timeout` is 50 ms
    When the model executes x = 1; search(query="a"); fetch(url="u")
    Then the status is {"status": "running", "run_id": 1}
    When 60 ms pass
    And the model executes x
    Then the status is failed with an error naming run 1 expired
    And the failed status lists the calls [{"tool_id": "search", "args": {"query": "a"}}]
    When the model executes x
    Then the status is {"status": "finished", "value": 1}

  Scenario: The default limits are the design's
    Then the default limits are 30 s of run time, 256 MiB, a recursion depth of 1000, 100 calls per execute, no call or idle timeout and 40,000 output characters
