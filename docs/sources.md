# 지식 근거 연결 API

source는 원본을 식별하는 불변 참조입니다. `uri`, `revision`, `label`, `created_by`를 저장하며 URI를 가져오거나 최신 여부를 확인하지 않습니다. URI는 `https:`, `file:`, `git:`, `urn:` 등 절대 URI scheme이 필요합니다. revision에는 commit SHA, 문서 버전, 캡처 시각 등 호출자가 확인한 식별자를 넣습니다. label은 사람이 읽을 이름입니다.

## source 생성과 조회

`POST /api/v1/sources` — contributor/reviewer/admin, `Idempotency-Key` 필수:

```json
{"uri":"https://example.org/research/report","revision":"2026-11-08","label":"실험 보고서"}
```

응답은 `{"data":{"id":"...","project_id":"...","uri":"...","revision":"...","label":"...","created_by":"..."},"sequence":1}` 형태입니다. URI는 최대 4096 bytes, revision 256 bytes, label 500 bytes이며 공백뿐인 값과 제어 문자는 거부합니다. `GET /api/v1/sources/{id}`는 reader를 포함한 같은 프로젝트 토큰으로 조회할 수 있습니다. 다른 프로젝트 ID는 404입니다.

source 수정·삭제 API는 없습니다. 같은 URI의 새 버전을 기록할 때 새 source를 만듭니다. 생성 내용이 같아도 다른 멱등성 키는 별도 레코드를 만듭니다. 같은 operation/principal/key와 같은 본문은 최초 응답을 재생하며, 다른 본문은 409 `idempotency_conflict`입니다.

## 지식에 첨부

`POST /api/v1/knowledge/{id}/sources` — contributor/reviewer/admin, `Idempotency-Key` 필수:

```json
{"expected_version":1,"expected_content_revision":1,"source_id":"생성된-source-id"}
```

첨부는 knowledge의 version과 content_revision을 각각 1 올리고 draft로 되돌립니다. 본문 content_author는 유지하며 첨부한 principal은 연결의 attached_by로 기록합니다. 본문 작성자와 현재 근거를 첨부한 principal 모두 해당 revision의 검토자가 될 수 없습니다. 이전 content_revision의 연결을 새 revision으로 복사한 뒤 source를 추가합니다. 이전 검토는 이전 content_revision에 남으며 새 내용을 승인할 수 없습니다. 일반 지식 본문 revision도 기존 source 집합과 attached_by를 복사하고 draft로 전환합니다. 근거를 이어받은 새 revision에서도 해당 첨부자의 검토는 차단합니다.

반환된 data에는 새 지식 상태와 `sources` 배열이 포함됩니다. `GET /api/v1/knowledge/{id}` 및 `?version=N`도 해당 이력의 정확한 content_revision에 연결된 source 배열을 반환합니다. 이전 이력의 연결은 바뀌지 않습니다.

낡은 expected_version은 409 `version_conflict`, 현재 version에서 다른 expected_content_revision은 409 `content_revision_conflict`입니다. 이미 현재 revision에 연결된 source를 새 키로 또 붙이면 409 `source_already_attached`입니다. 같은 첨부 요청의 멱등성 재시도는 최초 응답을 반환합니다. source 또는 knowledge가 다른 프로젝트에 있으면 404이며 변경이나 이벤트는 남지 않습니다.

한 지식 content_revision에는 최대 16개 source를 연결할 수 있으며 17번째 첨부는 409 `source_limit`입니다. 현재 첨부는 추가만 지원합니다. source 교체·제거·폐기는 아직 없으며 새 source를 추가해도 이전 source 연결은 유지합니다. 원문 가져오기, 해시 검증, URI 접근 권한 확인, 최신성 확인도 제공하지 않습니다.

## 승인과 업그레이드

schema_version=4는 source 및 revision별 연결 테이블을 추가합니다. 마이그레이션은 기존 지식 상태와 불변 이력을 수정하지 않으므로 이전 verified 지식은 유지됩니다. 업그레이드 후 새로 approved verdict를 기록하는 모든 knowledge revision에는 source 연결이 적어도 하나 있어야 합니다. 없으면 409 `sources_required`이며 검토는 pending 상태로 유지됩니다. 기존 draft 또는 pending review에도 이 새 승인 조건이 적용됩니다. task 승인에는 source 조건이 없습니다.

source 생성, 첨부, 멱등성 기록, 이벤트는 같은 프로젝트 잠금 아래 한 트랜잭션에서 커밋합니다. 철회된 토큰은 이전 성공 요청의 재생도 거부합니다. events에는 source/knowledge ID와 actor만 포함하며 URI, label, revision, 본문을 반복하지 않습니다. 이 API는 근거 참조의 존재를 확인하며 내용의 정확성을 자동 판정하지 않습니다.
