# Tasks

## 1. Answering an unknown outcome (`door`, `slack`)

- [ ] 1.1 Tell a post that may have been sent from one that was not: an unreadable 200 body, a timeout or read failure after sending, and an `ok` with no `ts` are unknown; a connect failure, a rate limit and a 5xx are not; add a loopback-server test that fails on today's code (A post whose outcome is unknown is not retried)
- [ ] 1.2 Map the unknown outcome to a non-transient `Error::Endpoint` beginning `post-outcome-unknown: `, with vectors for each scenario (A post whose outcome is unknown is not retried)
- [ ] 1.3 Archive the change into `openspec/specs/`
