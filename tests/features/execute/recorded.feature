Feature: Scripts a model sent in a live run give the status the model saw

  Background:
    Given a sandbox whose `output_chars` is 4000

  Scenario: claude-sonnet-5-5 on 2026-11-21 prints a pull request's check runs, then the failed run's log
    Given a host whose `get_pull_request` input schema is {"type": "object", "properties": {"number": {"type": "integer"}}, "required": ["number"]}
    And a host whose `get_pull_request` description is "Gets a pull request, with its head commit as head_sha."
    And a host whose `get_pull_request` answers {"number": 42, "title": "Cache check runs", "head_sha": "9f3c2e1"}
    And a host whose `list_check_runs` input schema is {"type": "object", "properties": {"commit": {"type": "string"}}, "required": ["commit"]}
    And a host whose `list_check_runs` description is "Lists a commit's check runs, each with an id, a name and a conclusion."
    And a host whose `list_check_runs` answers [{"id": 7, "name": "lint", "conclusion": "success"}, {"id": 8, "name": "test", "conclusion": "failure"}]
    And a host whose `get_log` input schema is {"type": "object", "properties": {"run_id": {"type": "integer"}}, "required": ["run_id"]}
    And a host whose `get_log` description is "Gets a check run's log as text."
    And a host whose `get_log` answers "running 12 tests\ntest cache::hit ... ok\ntest cache::miss ... FAILED\nerror: fixture ZEBRA-4417 is missing\n"
    When the model executes this script:
      """
      pr=get_pull_request(number=42)
      runs=list_check_runs(commit=pr["head_sha"])
      print(runs)
      """
    Then the status is {"status":"finished","stdout":"[{'conclusion': 'success', 'id': 7, 'name': 'lint'}, {'conclusion': 'failure', 'id': 8, 'name': 'test'}]\n","value":null}
    When the model executes this script:
      """
      print(get_log(run_id=8))
      """
    Then the status is {"status":"finished","stdout":"running 12 tests\ntest cache::hit ... ok\ntest cache::miss ... FAILED\nerror: fixture ZEBRA-4417 is missing\n\n","value":null}

  Scenario: claude-sonnet-5-5 on 2026-11-21 reads the host's refusal of a write from the traceback
    Given a host whose `write` input schema is {"type": "object", "properties": {"path": {"type": "string"}, "text": {"type": "string"}}, "required": ["path", "text"]}
    And a host whose `write` description is "Writes text to a file in the workspace."
    And a host that refuses `write` with "read-only workspace"
    When the model executes this script:
      """
      r = write(path="notes.txt", text="done")
      r
      """
    Then the status is {"calls":[{"args":{"path":"notes.txt","text":"done"},"tool_id":"write"}],"error":"Traceback (most recent call last):\n  File \"<python-input-0>\", line 1, in <module>\n    r = write(path=\"notes.txt\", text=\"done\")\n        ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~\nPermissionError: read-only workspace","status":"failed"}

  Scenario: claude-sonnet-5-5 on 2026-11-21 polls for a report the host answers after the execute wait
    Given a sandbox whose execute wait is 1000 ms and poll wait is 10000 ms
    And a host whose `fetch_report` input schema is {"type": "object", "properties": {"quarter": {"type": "string"}}, "required": ["quarter"]}
    And a host whose `fetch_report` description is "Fetches a quarter's report, with its reference code. It can take a while."
    And a host whose `fetch_report` answers only when told to
    When the model executes this script:
      """
      fetch_report(quarter="Q3")
      """
    Then the status is {"run_id":1,"status":"running"}
    When `fetch_report` answers {"quarter": "Q3", "reference": "PLUM-6630"}
    And the model polls run 1
    Then the status is {"status":"finished","value":{"quarter":"Q3","reference":"PLUM-6630"}}

  Scenario: claude-sonnet-5-5 on 2026-11-21 filters a log longer than the output budget down to its ERROR line
    Given a host whose `read_log` description is "Reads the service log, about 20,000 characters, one entry per line."
    And a host whose `read_log` answers a 20000 character log with "ERROR quota exceeded, code QX-5521" on its middle line
    When the model executes this script:
      """
      r=read_log()
      L=r if isinstance(r,list) else str(r).split("\n")
      [l for l in L if "ERROR" in l]
      """
    Then the status is {"status":"finished","value":["ERROR quota exceeded, code QX-5521"]}

  Scenario: claude-sonnet-5-5 on 2026-11-21 passes an argument that isn't a Python name with keyword unpacking
    Given a host whose `lookup` input schema is {"type": "object", "properties": {"user-id": {"type": "string"}}, "required": ["user-id"]}
    And a host whose `lookup` description is "Looks up a user by id, with their display name."
    And a host whose `lookup` needs `user-id` and then answers {"user-id": "u-17", "display_name": "Ondine Vasquez-Rho"}
    When the model executes this script:
      """
      lookup(**{"user-id":"u-17"})
      """
    Then the status is {"status":"finished","value":{"display_name":"Ondine Vasquez-Rho","user-id":"u-17"}}
