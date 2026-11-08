# Task-focused context

Schema 5 adds explicit project-scoped task → knowledge links. A link pins the
knowledge content revision, its exact immutable object-history version and source
references. The service never fetches source URIs. Each task permits 16 links.

Create a link with a contributor/reviewer/admin token, an `Idempotency-Key` header,
and `POST /api/v1/tasks/{task_id}/knowledge`:

```json
{
  "expected_task_version": 2,
  "run_id": "worker-run-17",
  "claim_epoch": 1,
  "knowledge_id": "knowledge-uuid",
  "knowledge_content_revision": 3
}
```

All fields are required; unknown fields are rejected. `expected_task_version`
and `knowledge_content_revision` must match current state. The caller must own
an active task lease for the exact run and claim epoch. Both objects must be the
correct kind in the token's project. Reader tokens cannot create links. Link
creation increments task `version`, preserves task `content_revision`, acceptance
text, content author and review validity, and appends one content-free event.
The response contains `data.task`, `data.link` and `sequence`.

Links are immutable. Repeating the same key and input replays the original
response; changing that input returns `idempotency_conflict`. A new key for an
already linked knowledge ID returns `knowledge_already_linked`. There is no
repin or removal endpoint in this version. Corrections can use a new knowledge
ID. Other conflicts include `version_conflict`, `content_revision_conflict`,
`claim_required`, `stale_claim`, `claim_expired`, `task_done` and `knowledge_limit`.
A revoked token cannot create or replay a link.

Read with `GET /api/v1/tasks/{task_id}/context?max_chars=8000`, or equivalently
`GET /api/v1/context?task_id={task_id}&max_chars=8000`. All project roles can read.
The latter form rejects mixed search/paging parameters. `max_chars` defaults to
8000 and must be between 512 and 32000. A single database read snapshot covers
objects, reviews and sources; reads append no events.

`items` are ordered as follows:

1. Current task, including its acceptance description, content revision and lease.
2. At most 16 compact task reviews for that exact current content revision,
   ordered by review ID. Each includes reviewer, requester, status, object version,
   subject revision and a direct lookup URL. `reviews_have_more` reports overflow.
3. Explicitly linked knowledge, ordered by knowledge ID. Its `data` is the pinned
   object snapshot with that revision's immutable `sources`. Each item includes
   a version-specific lookup URL, `pinned_content_revision`,
   `current_content_revision`, `pinned_version`, `current_version`, `linked_by`,
   `status_at_link`, `current_status`, `status_changed`, `content_changed` and
   `stale`. `stale` means content revision changed; review status changes can
   occur on the same content revision and are independently exposed.

Unlinked knowledge, unrelated tasks and messages are excluded. Pinned status is
historical: a pinned verified snapshot may now have `current_status: "draft"`.
Callers should inspect current trust metadata before using a finding.

`content_chars` is the exact Unicode character count of the serialized `items`
array, including brackets, commas and JSON escapes, and never exceeds
`max_chars`. The surrounding response metadata is outside this budget; this is
not a whole-response token bound. Whole items are retained as an ordered prefix.
If the mandatory task alone is oversized, `items` is empty, `incomplete` is true
and `lookup` identifies the task's direct endpoint. No truncated body is passed
off as a complete task.

Any budget omission or review overflow sets `incomplete`. `omitted_at_least` is
a lower bound; `omitted` contains at most eight identifiers and available direct
lookup URLs, never omitted bodies. More than 16 reviews requires direct lookups
or another query; this endpoint does not paginate reviews. `next_cursor` is
always null, so no cursor advances over content that was not returned.

Tests in `tests/task_context.rs` share the HTTP contract across SQLite and
PostgreSQL. The PostgreSQL test is ignored by default and uses
`TEST_DATABASE_URL` when explicitly enabled.
