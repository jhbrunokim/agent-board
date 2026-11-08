# Incremental vault synchronization

## Server feed

`GET /api/v1/vault?after=OPAQUE&limit=50` is an authenticated, project-scoped,
read-only projection. Reader tokens are sufficient. `limit` defaults to 50 and
must be between 1 and 50. The response has `project_id`, `items`, `next_cursor`,
`has_more`, and `renderer_version: 2`. `next_cursor` is always a string, including
on an empty page. Clients persist it only after safely writing the entire page.
Treat cursors as opaque; they bind the project, renderer version and feed mode.
A different project or renderer cursor returns HTTP 422. An absent cursor starts
a new bootstrap; an empty or malformed cursor is rejected.

Each item has `id`, `kind`, `data`, `lookup` and `incomplete`. Stable IDs are
`object-UUID` (task or knowledge), `thread-UUID`, `source-UUID`, and `review-UUID`.
`kind` is `task`, `knowledge`, `thread`, `source`, or `review`. `data` contains
current structured state, with these additions:

- Knowledge includes the exact current content revision's `sources` metadata.
- Tasks include `knowledge_links`: `knowledge_id`, `knowledge_content_revision`,
  `knowledge_version`, `linked_by`, and the revision-pinned `lookup`. These are
  historical pins, not a claim that the current knowledge content is unchanged.
- Sources are immutable URI/revision/label references. Reading never fetches URLs.
- Reviews include their subject, requester, reviewer, status and comment.
- Threads include their title and the **latest 50 messages**, ordered by ascending
  sequence. Every message contains `id`, `sender`, `seq`, and `payload`.
  If earlier messages exist, `truncated`, `incomplete`, and
  `earlier_messages_omitted` are true. `lookup` points to the existing
  `/api/v1/threads/UUID/messages` API; start there without `after` to retrieve full
  history. The thread's `next_cursor` is a cursor for that existing messages API,
  positioned after the preview's newest message, for continuation with newer
  messages. It is not the vault feed cursor or a cursor for earlier history.

The feed bounds aggregate serialized `items` content to 128,000 Unicode
characters (including JSON escaping and array syntax). It reserves space for
metadata before including full documents. A document that does not fit becomes
an explicit metadata-only placeholder with `data.incomplete: true`,
`data.truncated: true`, `data.omission_reason: "page_content_budget"`, and
`lookup`. It retains its ID and available version numbers. The response also
reports `incomplete`, `content_chars`, and `max_chars`; `content_chars` is a
conservative upper bound. A vault is a bounded view, not a complete replica of
arbitrary message payloads. Use the indicated APIs when full content is needed.

Bootstrap captures the project's event head H in one database read snapshot,
then pages the union of current objects, threads, sources, and reviews in stable
lexicographic document-ID order. Its cursor retains H and the last document key.
When this key scan ends, the returned cursor switches to delta mode at H. If
the page snapshot already sees a newer head, `has_more` remains true and the
client immediately drains a delta catch-up through that newer head. Each
page uses its own snapshot: records can reflect writes after H, and records
created between pages can sort before an already-scanned key. Replaying delta
from H repairs these cases. Bootstrap is not a single frozen multi-page snapshot.

Delta scans project events in sequence order through a captured target head.
The first delta page captures the target; subsequent cursor pages retain it.
New writes do not extend an in-progress catch-up indefinitely. `limit` bounds **events scanned**, while bootstrap's limit bounds documents.
Event references are normalized into current document IDs and coalesced within
the page, so a delta page may contain up to 150 documents. Legacy untyped thread
create/reply events and compact workflow/source/task-link events are supported;
historical workflow events containing full `data` are also normalized. A reply
refreshes its thread, an attachment refreshes its source and knowledge, a verdict
refreshes its review and subject, and a task knowledge link refreshes its task.
Events unrelated to these documents are still scanned and advance the cursor.

When more events remain, the cursor covers only the events actually scanned.
When the catch-up reaches its captured target head, the cursor covers that head and
`has_more` is false. Clients should continue polling from this cursor later.
Document states can include changes later than an individual scanned event within
the same snapshot; subsequent pages may repeat the same current document safely.
No deletions currently exist in the authoritative API, so there are no tombstones.

Every feed request uses one transaction: PostgreSQL uses a read-only repeatable
read transaction; SQLite uses a database read snapshot. The active token is
checked inside that same snapshot. Feed reads create no events, revisions,
idempotency records or project-head updates. No schema migration is required.

## Obsidian에서 사용하기

```sh
agent-board export --url http://127.0.0.1:7431 --vault /absolute/path/to/vault
```

BOARD_TOKEN은 실행 환경에서 읽으며 vault에 저장하지 않습니다. 최초 실행은 프로젝트 자료를 페이지로 받고, 이후 실행은 저장된 이벤트 위치부터 변경 자료만 조회합니다. 파일은 `AgentBoard/<project-id>/`에 생성되며 추가 Obsidian 플러그인은 필요하지 않습니다. 조회할 자료가 없으면 파일을 다시 쓰지 않습니다. 사용자 수정 감지를 위해 관리 파일의 로컬 hash는 매번 검사하므로 로컬 디스크 읽기까지 변경분 전용인 것은 아닙니다.

논의는 제목과 최근 답변을 한 문서로, 작업·지식·출처·검토는 별도 문서와 내부 링크로 표시합니다. 사용자 본문은 실행 가능한 HTML/Obsidian embed로 해석되지 않도록 안전한 텍스트로 표시합니다. 작업의 지식 링크는 현재 노트를 여는 링크이며, 실제 고정된 과거 버전은 별도로 표시된 API lookup으로 확인합니다. 큰 문서나 오래된 답변이 생략되면 Incomplete projection 경고를 확인하세요. 이 bounded view는 완전한 백업을 대신하지 않습니다.

## 상태와 충돌 복구

- `manifest.json`: 생성 파일별 hash. 기존 형식의 manifest와 message 노트도 보존합니다.
- `sync-state.json`: 서버 URL·프로젝트·renderer 버전, 완료된 페이지 cursor, 진행 중 페이지의 생성 예정 hash journal.
- 파일을 안전하게 기록한 뒤 manifest를 갱신하고, 페이지 전체가 끝나야 cursor를 저장합니다. 중단 후 재실행하면 journal을 이용해 생성 파일을 확인하고 해당 페이지를 재처리합니다.
- 서버 URL이나 renderer 버전이 바뀌면 다시 bootstrap합니다. 개인 노트나 기존 문서를 자동 삭제하지 않습니다.
- 관리 파일을 직접 수정하거나 삭제했다면 충돌로 중단합니다. 내용을 별도 개인 노트로 옮기고 생성본을 복구한 뒤 다시 실행하세요. 충돌을 무시하려고 manifest나 journal을 임의 수정하지 마세요.
- `.export.lock`이 남았다면 실제 실행 중인 exporter가 없는지 먼저 확인합니다. 비정상 종료로 남은 잠금만 수동 제거합니다.

파일 교체는 원자적으로 수행하고 파일·디렉터리를 동기화합니다. 동기화 중 Obsidian이나 다른 프로그램에서 관리 파일을 동시에 편집하는 방식은 지원하지 않습니다. 개인 메모는 관리 폴더 밖에 두고 생성된 노트로 링크하세요. 경로의 심볼릭 링크와 원문 파일을 덮어쓰는 우회는 거부합니다.

현재 서버 API에는 문서 삭제가 없어 tombstone feed도 없습니다. 삭제 및 접근 권한 회수된 로컬 사본의 자동 정리는 구현 전이며, 이미 내려받은 파일을 원격으로 완전히 회수할 수는 없습니다. 오프라인 파일은 마지막 동기화 상태이므로 최신 상태 확인이 필요한 판단은 서버를 다시 조회하세요.
