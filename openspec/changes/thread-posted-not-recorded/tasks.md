# Tasks

## 1. Telling a posted write apart (`slack`, `door`)

- [ ] 1.1 Add a `ThreadError` variant for a write that posted but was not recorded, carrying the key and an optional `ts`, and map the failed save of `threads.tsv` to it; the test is the finding's reproduction, a directory on `threads.tsv.tmp`, one write, which must answer posted-not-recorded with the ts (A write that posted but was not recorded says so)
- [ ] 1.2 Answer the same error for a Slack answer with no `ts`, a malformed `ts` and a refused `ts`, replacing `TransportError::Unavailable` and the plain `State` in `web_api.rs`; one test for each (A write that posted but was not recorded says so)
- [ ] 1.3 Map the variant in the door (`serve.rs`) to `posted-not-recorded`, not a retryable class, and keep a failed post's answers as they are; test both (A write that posted but was not recorded says so)
- [ ] 1.4 A retry must not post again: remove the directory, write again after the refusal, and show the first write posted once and the door added no post of its own (A write that posted but was not recorded says so)

## 2. Vectors and archive

- [ ] 2.1 Vectors for every scenario in `openspec/vectors.json` (A write that posted but was not recorded says so)
- [ ] 2.2 Archive the change into `openspec/specs/threads/spec.md`
