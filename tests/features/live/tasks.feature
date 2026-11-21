@live
Feature: Claude finishes tasks with only execute and poll

  Background:
    Given a sandbox whose `output_chars` is 4000

  Scenario: Claude chains a pull request, its commit's check runs and the failed run's log
    Given a host whose `get_pull_request` input schema is {"type": "object", "properties": {"number": {"type": "integer"}}, "required": ["number"]}
    And a host whose `get_pull_request` description is "Gets a pull request, with its head commit as head_sha."
    And a host whose `get_pull_request` answers {"number": 42, "title": "Cache check runs", "head_sha": "9f3c2e1"}
    And a host whose `list_check_runs` input schema is {"type": "object", "properties": {"commit": {"type": "string"}}, "required": ["commit"]}
    And a host whose `list_check_runs` description is "Lists a commit's check runs, each with an id, a name and a conclusion."
    And a host whose `list_check_runs` answers [{"id": 7, "name": "lint", "conclusion": "success"}, {"id": 8, "name": "test", "conclusion": "failure"}]
    And a host whose `get_log` input schema is {"type": "object", "properties": {"run_id": {"type": "integer"}}, "required": ["run_id"]}
    And a host whose `get_log` description is "Gets a check run's log as text."
    And a host whose `get_log` answers "running 12 tests\ntest cache::hit ... ok\ntest cache::miss ... FAILED\nerror: fixture ZEBRA-4417 is missing\n"
    When Claude answers "Why did the checks on pull request 42 fail? Quote the error line from the log." in 3 tries
    Then at least 2 of the 3 replies contain "ZEBRA-4417"

  Scenario: Claude reports why the host refused a tool call
    Given a host whose `write` input schema is {"type": "object", "properties": {"path": {"type": "string"}, "text": {"type": "string"}}, "required": ["path", "text"]}
    And a host whose `write` description is "Writes text to a file in the workspace."
    And a host that refuses `write` with "read-only workspace"
    When Claude answers "Write the text 'done' to notes.txt, then tell me whether it worked and, if not, the exact reason the tool gave." in 3 tries
    Then at least 2 of the 3 replies contain "read-only"

  Scenario: Claude polls a run that is still waiting on the host
    Given a sandbox whose execute wait is 1000 ms and poll wait is 10000 ms
    And a host whose `fetch_report` input schema is {"type": "object", "properties": {"quarter": {"type": "string"}}, "required": ["quarter"]}
    And a host whose `fetch_report` description is "Fetches a quarter's report, with its reference code. It can take a while."
    And a host whose `fetch_report` holds its answer {"quarter": "Q3", "reference": "PLUM-6630"} until the model polls
    When Claude answers "What is the reference code of the Q3 report?" in 3 tries
    Then at least 2 of the 3 replies contain "PLUM-6630"

  Scenario: Claude searches a log too long for the output budget inside the script
    Given a host whose `read_log` description is "Reads the service log, about 20,000 characters, one entry per line."
    And a host whose `read_log` answers a 20000 character log with "ERROR quota exceeded, code QX-5521" on its middle line
    When Claude answers "One line of the service log is an ERROR. What code does it give?" in 3 tries
    Then at least 2 of the 3 replies contain "QX-5521"

  Scenario: Claude calls a tool whose argument isn't a Python name with keyword unpacking
    Given a host whose `lookup` input schema is {"type": "object", "properties": {"user-id": {"type": "string"}}, "required": ["user-id"]}
    And a host whose `lookup` description is "Looks up a user by id, with their display name."
    And a host whose `lookup` needs `user-id` and then answers {"user-id": "u-17", "display_name": "Ondine Vasquez-Rho"}
    When Claude answers "What is the display name of the user with id u-17?" in 3 tries
    Then at least 2 of the 3 replies contain "Vasquez-Rho"
