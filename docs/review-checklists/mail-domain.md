# Mail domain and sync correctness review

Applies to any change to the database schema, sync deltas, op_log, threads, flags, message identity, or UIDVALIDITY handling.

Read first: [the data model](../architecture/data-model.md).

The guiding question: **will the user's mail be lost or corrupted?** Mistakes here are irreversible—the local cache is wiped and the message is already gone from the server.

## Checklist

1. **Identity.** Has a `uid` appeared where a `MessageId` belongs—especially in `op_log`, UI DTOs, the router, or the body cache? `(folder_id, uid)` is valid only with the current `uid_validity`; is that checked at the point of use?
2. **Deletions.** Can this code delete local data on `VanishedReport::Unknown`, or outside an `AuthoritativeSet` window? Is the `max(50, 20%)` threshold respected?
3. **Transactions.** Is the sync cursor written before the delta commits? Is the delta applied in one transaction? Does a writing transaction open as `BEGIN IMMEDIATE`? Does any transaction survive an `.await`?
4. **UIDVALIDITY.** What happens to pending operations on a change? Do they move to `needs_reresolve`, and do unresolved ones become `conflicted` and surface to the user rather than being dropped silently?
5. **Flags.** Does every copy of a message carry its own flags, and is `FlagScope` honored?
6. **Migrations already run.** Does the change edit a migration instead of adding one? A migration that has run on any machine is a record of what that database was put through: editing it changes nothing there, and the schema in the repository stops describing the schema people have. The symptom is a build that works on a fresh database and fails on every existing one. Does the new migration carry existing rows over, and is there a test that starts at the older version and writes what this build writes?
7. **Authoritative versus derived.** Have `op_log`, outbox, drafts, or settings landed in a database the code treats as recreatable? Do authoritative writes commit on a `synchronous = FULL` connection?
8. **Threads.** Is `ThreadKey` stable across merges? Is merging monotonic (lower id survives)?
9. **Time.** Are missing, malformed, and future `Date` values handled? Does time reach the rule as a value rather than being read from the system inside it?
10. **Sending.** Is `Message-ID` generated before the SMTP session opens? Is automatic retry forbidden on an indeterminate outcome?
11. **Recovery after process kill.** Is `sent_at` committed before the server is told, so a row the process died holding reads as possibly carried out rather than going out again as untried?

## Reporting

Cite file and line for every finding, most severe first. For each, give a concrete failure scenario—inputs and the resulting behavior—and the minimal fix. If nothing is found, say so; do not pad the list.
