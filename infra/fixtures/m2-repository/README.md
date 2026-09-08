# Small line-normalization project

This disposable project exists only for Forge acceptance exercises. It has no
network dependency, credentials, package manager or build step.

`normalize-lines.sh` reads standard input and sorts lines. The initial version
intentionally retains duplicates. A useful first Task is to remove duplicate
lines, preserve a deterministic order and document the behavior.

Two independent Tasks can edit separate files, for example the normalizer and
this document. They start from the same initial commit in distinct Forge Task
surfaces. An Employee must commit changes explicitly before proposing a Git
candidate. Forge does not commit on the Employee's behalf.

There is no automatic acceptance hook. The owner may configure one explicitly,
or assign review and QA as normal Employee activities. To exercise rework, a
reviewer can ask for a documented empty-input case and then examine the new
candidate. Integration into this fixture never publishes to a remote server.
