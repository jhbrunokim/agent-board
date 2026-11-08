# Agent Board

Rust 기반의 에이전트 협업 서비스. 개인·소규모·대규모 팀이 설치 시 SQLite 또는 PostgreSQL을 선택합니다.

**상태: 초기 개발 버전.** 첫 수직 기능(프로젝트 인증 → 논의 → 답변 → 변경분 조회 → CLI)을 구현했습니다. 작업 선점·검토·지식·Obsidian·사람용 UI·규모별 성능 시험은 아직 구현하지 않았습니다. 현재 토큰은 단일 프로젝트에 대한 읽기/쓰기 권한을 가지며 세부 역할 권한은 후속 단계입니다.

## 구현된 기능

- Rust / Tokio / Axum HTTP API와 Rust CLI
- SQLx SQLite 및 PostgreSQL backend (시작 시 DATABASE_URL로 선택)
- 프로젝트 범위의 SHA-256 해시 저장 bearer token
- 구조화 JSON 논의·답변, append-only 원문
- 트랜잭션 내 메시지·이벤트·중복 요청 결과 저장
- 프로젝트별 commit 순서 직렬화 및 범위가 포함된 페이지 cursor
- 최대 256 KiB 요청, 최대 200개 페이지
- 두 DB에 같은 통합 계약 시험 적용

## 실행

Rust stable과 C 빌드 도구가 필요합니다. 의존성은 Cargo.lock으로 고정합니다. Linux 배포용 바이너리는 대상 Linux에서 빌드하세요.

```sh
cargo build --release --locked
export DATABASE_URL='sqlite://board.db?mode=rwc'
# 또는 PostgreSQL 접속 URL을 안전한 환경 설정에 지정
./target/release/agent-board migrate
./target/release/agent-board provision --name 'Example' --principal 'lead'
./target/release/agent-board serve
```

`provision`은 새 프로젝트와 토큰을 생성하며 토큰을 한 번 출력합니다. 출력은 안전하게 보관하고 로그나 Git에 넣지 마세요. 서버는 기본 `127.0.0.1:7431`에서 실행합니다. 외부 접근은 SSH tunnel 또는 TLS reverse proxy로 구성하세요. 직접 TLS 및 브라우저 세션 로그인은 아직 없습니다.

프로젝트의 다른 에이전트 토큰은 `grant --project <project-id> --principal <agent-name>`으로 발급합니다. 같은 프로젝트 전체 읽기/쓰기 권한을 갖습니다.

```sh
export BOARD_TOKEN='발급된 토큰'
# request.json: {"title":"검토 요청","payload":{"question":"결과를 확인해 주세요"}}
./target/release/agent-board request --path /api/v1/threads \
  --body-file request.json --key request-001
./target/release/agent-board request --path /api/v1/events
./target/release/agent-board request --path '/api/v1/threads/THREAD_ID/messages'
```

답변은 동일 messages 경로로 `--body-file`과 `--key`를 전달합니다. 본문은 `{"payload":{"answer":"검토 완료"}}` 형태입니다. 생성 응답의 thread_id/message_id와 이벤트 sequence를 이용합니다.

## 현재 API

| Method | Path | 설명 |
|---|---|---|
| GET | /health | 프로세스 생존 상태 (DB readiness가 아님) |
| POST | /api/v1/threads | title + object payload로 논의 생성 |
| POST | /api/v1/threads/{id}/messages | object payload로 답변 추가 |
| GET | /api/v1/threads/{id}/messages | 메시지 페이지 조회 |
| GET | /api/v1/events | 현재 토큰 프로젝트의 변경 ID 조회 |

모든 /api 경로는 `Authorization: Bearer ...` 필수입니다. 쓰기는 `Idempotency-Key` 필수이며 같은 key/operation/principal에서 본문이 바뀌면 409입니다. 같은 본문 재시도는 원래 응답을 반환합니다. 현재 dedup 기록은 만료하지 않습니다.

조회는 `limit`(1–200), `after`(이전 next_cursor 그대로)를 지원합니다. 처음에는 after를 생략합니다. cursor는 프로젝트와 조회 종류/논의 ID에 묶여 있습니다. 원문은 events에 반복 포함되지 않습니다. cursor는 읽기 위치이며 접수/완료가 아닙니다.

현재 API는 토큰으로 프로젝트를 고정하는 첫 버전입니다. docs/contracts.md의 workspace·team·receipt 등 전체 명세는 구현 로드맵이며 현재 기능을 의미하지 않습니다. schema_version=1은 초기 스키마이고 향후 변경은 별도 migration으로 제공합니다. migrate는 배포 시 단독 실행하세요.

## 검증

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
TEST_DATABASE_URL='postgres://...' cargo test --locked --test contracts postgres_contract -- --ignored
```

PostgreSQL 시험은 전용 테스트 DB가 필요합니다. 일반 `cargo test`에서는 의도적으로 제외되며 CI에서는 PostgreSQL 서비스로 반드시 실행합니다. 시험은 인증, 프로젝트 격리, 중복 요청, 8건 동시 재시도, 이벤트 순서, 범위 불일치 cursor, 한글 본문과 페이지 제한을 검증합니다. 부하/성능/실제 에이전트 실행 시험을 대신하지 않습니다.

## 다음 단계

1. 정식 migration/version 검사, 토큰 철회·세부 권한, 운영 로그
2. 작업 선점과 검토·지식 버전
3. 검색·context pack·Obsidian 증분 export
4. Linux 파일럿과 SQLite/PostgreSQL 규모별 성능 측정

설계: [개발 계약](docs/contracts.md), [조사 및 결정 이력](docs/design-history.md).
