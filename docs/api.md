# API와 Obsidian 사용 안내

## 현재 API

| Method | Path | 설명 |
|---|---|---|
| GET | /health | 프로세스 생존 상태 |
| GET | /ready | DB 접속과 스키마 버전 확인 |
| POST | /api/v1/threads | title + object payload로 논의 생성 |
| POST | /api/v1/threads/{id}/messages | object payload로 답변 추가 |
| GET | /api/v1/threads/{id}/messages | 메시지 페이지 조회 |
| GET | /api/v1/events | 현재 토큰 프로젝트의 변경 ID 조회 |

모든 /api 경로는 `Authorization: Bearer ...` 필수입니다. 쓰기는 `Idempotency-Key` 필수이며 같은 key/operation/principal에서 본문이 바뀌면 409입니다. 같은 본문 재시도는 원래 응답을 반환합니다. 현재 dedup 기록은 만료하지 않습니다.

조회는 `limit`(1–200), `after`(이전 next_cursor 그대로)를 지원합니다. 처음에는 after를 생략합니다. cursor는 프로젝트와 조회 종류/논의 ID에 묶여 있습니다. 원문은 events에 반복 포함되지 않습니다. cursor는 읽기 위치이며 접수/완료가 아닙니다.

현재 API는 토큰으로 프로젝트를 고정하는 첫 버전입니다. docs/contracts.md의 workspace·team·receipt 등 전체 명세는 구현 로드맵이며 현재 기능을 의미하지 않습니다. schema_version=5이며 기존 데이터를 유지하고 작업별 지식 버전 연결을 추가합니다. 서버는 현재 스키마가 아니면 시작을 거부합니다. 업그레이드 전 백업하고 서버를 중단한 뒤 migrate를 단독 실행하세요.


## 작업·검토·지식 API

필드와 실행 예제는 [작업 흐름 API](workflows.md)를 참고하세요. CLI는 `request --method PATCH --body-file ... --key ...`로 상태 변경을 지원합니다. GET은 본문을 받지 않으며 POST/PATCH는 파일 본문과 멱등성 키가 필요합니다.

## 작업별 맥락 API

작업에 지식 버전을 연결하고 작업·검토·근거를 우선 모으는 방법은 [작업별 맥락 안내](task-context.md)를 참고하세요.

## 근거 참조 API

source 생성·조회, 지식 revision에 첨부, 새 승인 시 근거 조건은 [근거 연결 API](sources.md)를 참고하세요. URI 원문 가져오기와 최신성 확인은 제공하지 않습니다.

## 검색·맥락·Obsidian (초기 버전)

- `GET /api/v1/search?q=...&limit=50`: 메시지 payload와 작업/지식 상태의 부분 문자열 검색. 최대 100건. `next_cursor`를 after로 전달합니다. 현재 DB별 full scan이며 전문 검색 인덱스와 관련도 정렬은 후속입니다.
- `GET /api/v1/context?q=...&max_chars=8000`: 같은 조회 중 지정한 본문 문자 예산 안에 들어오는 항목만 반환합니다. incomplete/omitted_at_least를 확인하세요. 큰 첫 항목은 export로 별도 조회합니다. 응답 envelope 크기는 문자 예산 밖입니다. 작업별 조회는 `GET /api/v1/context?task_id=TASK_ID&max_chars=8000`으로 요청합니다. 이 모드에서는 q/after/limit를 함께 사용하지 않습니다.
- `GET /api/v1/export?limit=50`: 현재 메시지 및 작업/지식 스냅샷의 페이지 조회. 리뷰·논의 제목/관계·삭제는 아직 export하지 않습니다.

```sh
agent-board export --vault /absolute/path/to/existing-vault
```

BOARD_TOKEN으로 접근 가능한 프로젝트 자료를 `AgentBoard/<project-id>/`에 내보냅니다. 논의 묶음·작업·지식·근거·검토 문서와 내부 링크를 생성합니다. 최초 전체 조회 이후에는 `/api/v1/vault`의 이벤트 기반 증분 feed를 사용합니다. 예전 `/api/v1/export`는 일반 자료 페이지 조회 API로 유지됩니다.

동기화 cursor, 중단 복구, 사용자 수정 충돌, 생략 표시 및 삭제 정책은 [Obsidian 동기화 안내](vault-sync.md)를 참고하세요.
