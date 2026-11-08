# 작업·검토·지식 API (schema v2)

모든 경로는 프로젝트 토큰을 사용하며 쓰기에 Idempotency-Key가 필요합니다. 응답은 `{data: object, sequence: number}`, 조회는 `{data: object}`입니다. 충돌 시 409, 다른 프로젝트의 ID는 404입니다. 모든 필드는 문서화된 것만 허용합니다.

## 작업

- POST /api/v1/tasks: `{ "title": "작업", "description": "내용과 완료 기준" }`
- GET /api/v1/tasks/{id}: 현재 스냅샷. `?version=1`은 불변 과거 스냅샷.
- POST /api/v1/tasks/{id}/claims: `{ "expected_version": 1, "expected_epoch": 0, "run_id": "run-1", "lease_seconds": 60 }`
- PATCH /api/v1/tasks/{id}: `{ "expected_version": 2, "run_id": "run-1", "claim_epoch": 1, "status": "in_review" }`

claim은 1~3600초이며 활성 동일 소유자/run/epoch는 갱신, 다른 소유자는 거부합니다. 만료 후 획득은 새 epoch를 발급합니다. 상태/수정에는 유효 lease와 현재 version이 모두 필요합니다. title/description 변경 시 content_revision이 증가합니다. 상태는 open/in_progress/in_review/blocked/done이며 done에는 현재 콘텐츠의 독립 승인 필요. open은 lease 해제, done은 종료 상태입니다. blocked 사유의 별도 필드와 세부 전이 정책은 후속 작업입니다.

## 검토

- POST /api/v1/reviews: `{ "subject_kind": "task", "subject_id": "UUID", "subject_revision": 1, "expected_version": 2, "reviewer": "reviewer" }`
- GET /api/v1/reviews/{id}: 현재 판정과 대상 콘텐츠 revision.
- POST /api/v1/reviews/{id}/verdicts: `{ "expected_version": 1, "verdict": "approved", "comment": "근거 확인" }`

subject_kind는 task/knowledge, verdict는 approved/rejected. 지정 reviewer의 프로젝트 토큰만 판정 가능하며 요청자·작성자·작업 담당자의 자기 검토를 거부합니다. 내용이 바뀌면 과거 검토는 남지만 판정 요청은 stale_review로 거부되고 과거 승인으로 새 내용을 완료할 수 없습니다. 상태/lease 변경은 콘텐츠 revision을 바꾸지 않습니다. 독립성은 인증 principal 기준이며 실제 사람/모델의 독립성 자체를 보증하지 않습니다.

## 지식

- POST /api/v1/knowledge: `{ "title": "개념", "body": "내용과 근거" }`
- GET /api/v1/knowledge/{id}: 현재 스냅샷, `?version=n`으로 이력 조회.
- POST /api/v1/knowledge/{id}/revisions: `{ "expected_version": 2, "title": "개념", "body": "수정된 내용" }`

초안은 draft, 독립 approved 판정 시 verified, 수정 시 다시 draft입니다. 버전별 본문을 보존합니다. 검증은 해당 내용의 팀 검토 상태이며 제품 인증을 뜻하지 않습니다. 구조화된 source 레코드·증거 파일·검색·목록 및 Obsidian export는 아직 후속 개발 범위입니다. 현재 근거는 body/comment에 기록합니다.

변경 이벤트는 ID·유형·순서만 제공하며 긴 본문을 반복 전달하지 않습니다. CLI는 PATCH에 `request --method PATCH --path ... --body-file ... --key ...`를 사용합니다.
