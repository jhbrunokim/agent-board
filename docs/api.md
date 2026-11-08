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

현재 API는 토큰으로 프로젝트를 고정하는 첫 버전입니다. docs/contracts.md의 workspace·team·receipt 등 전체 명세는 구현 로드맵이며 현재 기능을 의미하지 않습니다. schema_version=3이며 v1/v2 데이터를 유지하고 토큰 권한·철회 정보를 추가합니다. 서버는 현재 스키마가 아니면 시작을 거부합니다. 업그레이드 전 백업하고 서버를 중단한 뒤 migrate를 단독 실행하세요.


## 작업·검토·지식 API

필드와 실행 예제는 [작업 흐름 API](workflows.md)를 참고하세요. CLI는 `request --method PATCH --body-file ... --key ...`로 상태 변경을 지원합니다. GET은 본문을 받지 않으며 POST/PATCH는 파일 본문과 멱등성 키가 필요합니다.

## 검색·맥락·Obsidian (초기 버전)

- `GET /api/v1/search?q=...&limit=50`: 메시지 payload와 작업/지식 상태의 부분 문자열 검색. 최대 100건. `next_cursor`를 after로 전달합니다. 현재 DB별 full scan이며 전문 검색 인덱스와 관련도 정렬은 후속입니다.
- `GET /api/v1/context?q=...&max_chars=8000`: 같은 조회 중 지정한 본문 문자 예산 안에 들어오는 항목만 반환합니다. incomplete/omitted_at_least를 확인하세요. 큰 첫 항목은 export로 별도 조회합니다. 응답 envelope 크기는 문자 예산 밖입니다. 아직 작업 관계 기반 자동 우선순위는 없습니다.
- `GET /api/v1/export?limit=50`: 현재 메시지 및 작업/지식 스냅샷의 페이지 조회. 리뷰·논의 제목/관계·삭제는 아직 export하지 않습니다.

```sh
agent-board export --vault /absolute/path/to/existing-vault
```

BOARD_TOKEN으로 접근 가능한 프로젝트 자료만 `AgentBoard/<project-id>/`에 내보냅니다. 토큰은 vault에 저장하지 않습니다. 원본 ID 기반 파일명, YAML 속성, 구조화 본문을 가진 Markdown을 생성합니다. 처음에는 Obsidian에서 vault를 열고 관리 폴더를 탐색하세요. 백링크/논의별 묶음과 자연어 설명 화면은 후속 개선입니다.

수동 pull 방식이며 전체 ID 페이지를 조회하되 변경된 파일만 다시 씁니다. 파일 hash가 변경됐다면 사용자 수정으로 보고 원본을 보존한 채 실패합니다. 개인 노트/.obsidian은 수정하지 않습니다. 동시 export를 막는 `.export.lock`이 남아 있으면 실행 중인 exporter가 없는지 확인한 뒤 수동 삭제합니다. 삭제/권한 회수된 로컬 복사본의 자동 정리는 아직 지원하지 않습니다. 페이지 사이에 변경이 발생할 수 있으므로 전체 export가 동일 시점의 snapshot이라고 가정하지 마세요. 안정된 상태에서 다시 실행하면 현재 버전으로 수렴합니다.

