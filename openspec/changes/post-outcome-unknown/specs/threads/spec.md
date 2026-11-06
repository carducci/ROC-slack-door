## ADDED Requirements

### Requirement: A post whose outcome is unknown is not retried

When the door cannot tell whether Slack posted a message, it MUST answer the write as an
endpoint error whose message begins `post-outcome-unknown: `, not as unavailable, so no retry
overlay posts the text a second time. A post that failed before it was sent SHALL stay
unavailable.

#### Scenario: Slack's answer cannot be read

- **WHEN** Slack answers a post with HTTP 200 and a body that is not valid JSON
- **THEN** the write is answered as an endpoint error beginning `post-outcome-unknown: `, and is not unavailable

#### Scenario: The request times out after it was sent

- **WHEN** a post's request was sent and Slack does not answer within the timeout
- **THEN** the write is answered as an endpoint error beginning `post-outcome-unknown: `

#### Scenario: The post failed before sending

- **WHEN** the door cannot connect to Slack, so nothing was sent
- **THEN** the write is answered as unavailable, and a retry may apply

#### Scenario: Slack answers ok with no timestamp

- **WHEN** Slack answers a post `ok` with no `ts`
- **THEN** the write is answered as an endpoint error beginning `post-outcome-unknown: `
