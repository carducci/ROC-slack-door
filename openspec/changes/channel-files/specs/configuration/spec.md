## ADDED Requirements

### Requirement: A channel lists the files it accepts

A channel's configuration MAY list the media types it accepts and a size limit in bytes. A
channel with no list SHALL accept no files. A listed channel with no size limit MUST be refused
at start, naming the channel. The door SHALL refuse to start when a listed type is not one
`ikigai-sniff` can detect, naming the type.

#### Scenario: Images only

- **WHEN** the host lists `image/png`, `image/jpeg`, `image/gif` and `image/webp` with a 10 MB limit for `fixme`
- **THEN** `fixme` accepts those four types up to 10 MB, and `gonk-chat` accepts none
