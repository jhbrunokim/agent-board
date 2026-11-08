# Agent Board — 개발 계약 v1

작성: 2026-11-08. 개발 전 명세이며 구현·부하 시험 결과가 아니다. 이전 agent-board-design.md의 언어/DB 비교에 관한 잠정안을 대체한다.

## 1. 확정 기준과 구현 기본안

사용자 확정: Rust 단일 구현, Linux 실행, 개인/소규모/대규모 팀의 범용 서비스, 설치 단위로 SQLite 또는 PostgreSQL 선택, 에이전트 중심 통신, 사람의 판단에 필요한 설명, Obsidian 조회. Go 구현·언어 비교는 하지 않는다.

구현 기본안: Tokio/Axum, Rust CLI, HTTP JSON v1, SSE 변경 알림, SQLx 기반 DB별 구현. crate 버전·MSRV는 구현 시 지원 상태를 확인하고 lockfile로 고정한다. DB 종류는 시작 시 고정하며 운영 중 자동 전환하지 않는다. 두 DB는 기능 동등성을 목표로 한다. 규모별 실제 수용량은 후속 시험으로 확인한다.

기존 설계의 객체·권한·원본 보존·토큰 절약 원칙은 유지한다. 이 문서는 그 실행 계약을 구체화한다.

## 2. 모듈 경계

```text
crates/
  domain/          # ID, 상태, 검증 규칙, 권한 정책, 업무 오류
  application/     # 요청/답변/작업/검토/지식 command와 query
  storage/         # 업무 단위 저장소 계약과 공통 계약 시험
  storage-sqlite/  # SQL, transaction, migration, 검색
  storage-postgres/# SQL, transaction, migration, 검색
  api/             # HTTP 인증/입력 검증/응답/에러 매핑/SSE
  cli/             # API client, export/import, 관리자 명령
  projection/      # context pack, Obsidian Markdown 렌더링
  server/          # 설정, 실행, background jobs, 관측성
```

domain은 HTTP/SQL/Obsidian에 의존하지 않는다. storage 계약은 테이블별 CRUD 나열보다 commit_message, claim_task, submit_review 같은 원자적 업무 연산을 노출한다. application은 여러 독립 repository 호출을 묶어 원자성이 있는 것처럼 취급하지 않는다. 정본 변경·이벤트·검색 색인·멱등성 결과를 동일 DB transaction에 저장한다. 파일 export는 커밋 후 수행하며 실패해도 원본 쓰기의 성공을 취소하지 않는다.

## 3. 데이터와 제약

| 테이블 그룹 | 필수 관계/규칙 |
|---|---|
| workspaces, projects, teams | 프로젝트/팀은 workspace 소속, 다대다 project_teams |
| principals, memberships, tokens | 인증 주체와 역할·팀 membership 분리, 원문 토큰 저장 금지 |
| threads, recipients, messages | thread는 project 소속, recipients에 팀별 접수, message는 불변 |
| tasks, task_claims | task version으로 수정 충돌, claim epoch로 만료된 소유자의 쓰기 방지 |
| reviews, review_verdicts | target ID와 revision에 고정, reviewer identity와 근거 보존 |
| knowledge, knowledge_revisions | stable ID와 불변 revision 분리, 현재 검토 상태 별도 |
| sources, relations | 출처 버전, typed 관계, 양쪽 workspace 일치 검증 |
| project_events, event_heads | 프로젝트별 순서와 현재 head, commit 순서에 안전한 cursor |
| checkpoints, idempotency | 소비자별 처리 위치, 재시도 결과 및 payload hash |
| export_manifests, import_runs | 실행 결과, 파일별 원본 ID/version/hash와 실패 기록 |

ID는 API에서 UUID 문자열, 시각은 UTC RFC3339다. 본문은 UTF-8. 이름/slug 변경은 ID를 바꾸지 않는다. 상태는 문자열 enum, 허용 값과 null 여부를 DB 제약 및 domain에서 검사한다. SQLite/PG 내부 표현은 달라도 API 정규화는 동일하다. project_id 포함 복합 FK/unique 제약으로 다른 프로젝트의 객체를 잘못 연결하지 못하게 한다. 다형 관계는 transaction 안에서 존재/권한/종류를 검증한다.

작업 상태: open → in_progress → in_review → done. blocked는 reason과 이전 상태를 저장하며 해소 후 허용 상태로 돌아간다. cancelled는 사유 필요. thread 해결과 task 완료는 독립이다. 지식 새 revision은 기존 verified 판정을 상속하지 않는다. decision은 version에 고정된 review/decision 기록이며 사용자 정의 기준 문서의 권위를 자동 대체하지 않는다.

## 4. 동시성·원자성

### 메시지 작성

1. 인증/범위/입력 제한 검사.
2. transaction 시작, idempotency key 확보 및 기존 결과 확인.
3. project event head 잠금/증가, message와 관계 저장.
4. 이벤트 및 검색 색인 저장, 응답과 dedup 기록 저장, commit.
5. commit 후 알림 발행. 알림 손실은 events 조회로 복구.

### 이벤트 순서

DB sequence/autoincrement 발급 순서가 항상 commit 순서는 아니므로 단순 MAX(id) cursor에 의존하지 않는다. 프로젝트별 event_heads 행을 짧게 잠그고 이벤트 번호를 transaction 안에서 할당한다. PostgreSQL은 해당 행 lock, SQLite는 짧은 write transaction으로 직렬화한다. 같은 프로젝트의 write 순서를 보장하며 다른 프로젝트의 PostgreSQL 쓰기는 별도로 진행할 수 있다. 대형 프로젝트 head 경합은 시험 항목이다.

API cursor는 project_id+sequence+filter fingerprint로 범위를 제한한다. 여러 프로젝트 조회는 프로젝트별 cursor 묶음이고 전역 엄격 순서를 보장하지 않는다. 권한 밖 이벤트는 반환하지 않으며 페이지 스캔 종료 위치와 실제 전달 항목을 구분한다. 읽기 cursor는 접수/처리 완료를 의미하지 않는다. 오래된 cursor에는 410 cursor_expired와 snapshot 경로를 반환한다.

### 작업 선점

claim은 owner/run, server-clock 기준 lease_until, 단조 증가 epoch를 가진다. 최초 획득/만료 후 재획득은 조건부 update로 한 요청만 성공한다. 활성 다른 owner는 409 task_claimed. 작업 쓰기는 expected_version과 claim epoch 둘 다 확인한다. lease 갱신도 epoch가 맞아야 한다. 네트워크 단절 후 예전 owner는 결과 제안/근거를 별도 메시지로 보낼 수 있지만 task 상태를 변경할 수 없다. claim은 코드 파일 접근 권한이나 Git lock을 대신하지 않는다.

### 재시도와 오류

write API에는 Idempotency-Key를 요구한다. 범위는 authenticated principal+HTTP operation+project. 정규화된 요청 hash가 동일하면 저장된 성공 응답 재생, 다르면 409 idempotency_conflict. DB busy/deadlock은 bounded retry 후 503 retryable로 반환한다. 저장된 결과는 기본 7일 보관(운영 설정)이며 기간 이후 중복 방지를 요구하는 외부 원문은 별도 unique source ID를 사용한다. 요청 취소/timeout은 미커밋 transaction을 rollback하고 성공 여부가 모호하면 같은 key로 확인한다.

## 5. HTTP v1 계약

목록 기본 limit=50, 최대 200. 페이지 응답은 items, next_cursor, snapshot_version을 가진다. 큰 본문은 명시적 상세 조회다. 인증 주체를 payload sender로 덮어쓸 수 없다.

| 경로 | 의미 | 주요 입력 |
|---|---|---|
| POST /api/v1/projects/{p}/threads | 논의 시작 | kind,title,recipients,payload,human_view? |
| POST /api/v1/threads/{id}/messages | 답변/정정 | parent_id?,correction_of?,payload |
| POST /api/v1/threads/{id}/receipts | 수신팀 접수 | recipient_team_id |
| GET /api/v1/projects/{p}/events | 변화 조회 | after,limit |
| GET /api/v1/projects/{p}/stream | SSE 알림 | Last-Event-ID |
| POST /api/v1/projects/{p}/tasks | 작업 등록 | title,acceptance,relations |
| POST /api/v1/tasks/{id}/claims | 작업 선점 | run_id,requested_lease_seconds |
| PATCH /api/v1/tasks/{id} | 상태/내용 변경 | expected_version,claim_epoch,patch |
| POST /api/v1/reviews | 검토 요청 | target_type,id,revision,reviewer |
| POST /api/v1/reviews/{id}/verdicts | 판정 기록 | expected_version,verdict,evidence |
| POST /api/v1/projects/{p}/knowledge | 지식 초안 | title,payload,sources |
| POST /api/v1/knowledge/{id}/revisions | 새 지식 버전 | expected_version,payload,sources |
| GET /api/v1/projects/{p}/search | 검색 | q,kind,status,limit |
| POST /api/v1/projects/{p}/context-packs | 작업 맥락 | task_id?,after?,max_chars |
| PUT /api/v1/projects/{p}/checkpoints/{consumer} | 처리 위치 기록 | expected_version,cursor |
| GET /api/v1/projects/{p}/export | 버전별 export 자료 | after?,snapshot?,limit |

GET 상세 경로는 각 객체 /{id}에 제공한다. 내부 ID를 알아도 권한 검사를 통과해야 한다. 미허용 객체는 존재를 누설하지 않는 404. 상태 충돌은 409, body 제한 초과는 413, 검증 오류는 422, 과부하는 429/503+Retry-After. 에러는 code, message, retryable, request_id, details(권한 내 정보)다.

schema_version은 프로토콜 해석 버전이고 entity version은 수정 충돌 버전이다. 지원하지 않는 major schema는 거부한다. 사용자 정의 payload는 namespace와 크기 상한을 갖추되 핵심 상태 전이를 우회하지 못한다.

## 6. 메시지와 사람용 표현

```json
{
  "schema_version": 1,
  "kind": "review.request",
  "correlation_id": "<uuid>",
  "payload": {
    "target": {"type": "task", "id": "<uuid>", "revision": 7},
    "criteria": ["API 응답 계약 충족", "재시도 시 중복 없음"],
    "evidence": [{"source_id": "<uuid>", "revision": "<commit>"}]
  },
  "human_view": {
    "title": "변경 검토 요청",
    "summary": "재시도 처리 변경을 검토해 주세요.",
    "uncertainties": ["운영 환경 부하 시험 미실행"]
  }
}
```

human_view는 단순 상태 이벤트에 선택 사항이며 사람의 결정 요청에는 필수다. 권고·선택지·영향·출처가 필요한 decision.request는 별도 schema로 강제한다. 원본 payload가 사실 기준이고 human_view와 모순되면 검증 실패/검토 대기로 처리한다. 생성 설명이 검증/승인 증거를 대신하지 않는다. 도구가 처리 가능한 ack/lease/retry는 모델 prompt에 넣지 않는다.

## 7. 두 DB 구현 계약

SQLite: 로컬 디스크, WAL, foreign_keys 활성, 한정된 connection pool, 짧은 write transaction, busy timeout과 제한 재시도. PostgreSQL: 한정된 pool, row lock/조건부 update, 동일 원자성·durability 보장. 검색은 DB별 구현하되 필터·권한·ID lookup·pagination 의미를 같게 하고 relevance 순위의 완전 일치는 보장하지 않는다.

DB별 migration 디렉터리를 두고 schema metadata에 version/checksum을 저장한다. startup migration은 운영자가 명시적으로 실행하도록 하고 서버 시작과 별개 명령으로 제공한다. 두 backend의 동일 계약 시험 통과가 릴리스 조건이다. 지원하지 않는 backend 기능을 사용자에게 조용히 숨기지 않는다.

SQLite→PostgreSQL 전환: maintenance/read-only → source backup → 새 target에 versioned export import → ID/FK/건수/hash/event head 검증 → API 읽기/쓰기 smoke test → 서버 설정 전환. 전환 후 target에 새 쓰기가 생기면 source로 단순 되돌리지 않는다. 롤백에는 target 변경 export 또는 전환 후 쓰기 보존 계획이 필요하다. reverse migration도 같은 계약을 따른다. DB 설정 변경은 데이터 이동 명령이 아니다.

## 8. Obsidian와 context 조회

server export는 권한 있는 자료와 source_version을 제공한다. 사용자 CLI가 vault의 AgentBoard/ 관리 영역만 갱신한다. immutable ID 기반 파일명과 읽기 쉬운 링크 별칭을 사용한다. vault의 Notes/ 및 .obsidian은 소유하지 않는다. manifest와 hash로 사용자의 수정 여부를 확인하고 충돌 시 원본 파일을 보존한다. 출력 파일 단위 원자 교체 후 manifest/cursor를 커밋한다. 재실행은 idempotent하다.

렌더링은 구조화 상태→표, 논의→원문/결론/근거, 결정→선택지/판정/버전으로 수행한다. LLM 요약은 기본 경로에서 제외한다. offline/마지막 성공 동기화 시각을 인덱스에 표시한다. Obsidian에서 status를 편집해도 서버 승인으로 처리하지 않는다.

context pack은 권한 필터 적용 후 기준 변경·내 작업·미해결 수신 요청·관련 지식 순으로 담는다. max_chars 상한, omitted 항목 수, incomplete, source versions, next lookup을 제공한다. DB/문서 전체를 복제하지 않는다. token estimate는 tokenizer 정보가 있을 때만 표시한다.

## 9. 성능과 초기 제한

초기 기본값은 검증 목표이며 실측 후 조정한다: JSON request 최대 256 KiB, 일반 페이지 최대 200개, context 최대 32,000자, 알림 queue 연결당 최대 64개 ID. 긴 증거는 별도 출처로 참조한다. 모든 제한은 서버가 enforce하며 초과 시 명시적으로 응답한다. 큰 DB/파일 작업은 Tokio I/O executor를 blocking하지 않도록 격리한다. bounded queue와 backpressure는 기본 규칙이다.

Rust 단일 서버에서 연결 50/500/5,000/20,000, 고정 총 RPS/연결당 고정 RPS, 읽기/쓰기 90:10 및 50:50을 시험한다. 두 DB를 동일 durability·인증·데이터 크기·총 자원 예산으로 비교한다. warm/cold cache, slow consumer, reconnect, 동일 task 경합, backup/import 부하를 포함한다. p95/p99, 처리량, 전체 메모리와 connection 증가분, DB 대기, 오류·데이터 유실을 기록한다. 20,000은 한계 탐색이며 지원 보장이 아니다.

## 10. 구현 단계와 완료 기준

1. domain/config/migration과 두 backend 테스트 골격: 같은 fixture에서 권한·FK·unique·rollback 확인.
2. 논의 생성→답변→변경 조회→CLI의 첫 수직 기능: 두 DB 모두 durable commit, 중복 재시도, commit 순서 안전성 검증.
3. task claim/review/knowledge: epoch·버전 경합·검토 무효화·만료 복구 검증.
4. 검색/context/Obsidian: 정보량 제한·권한·한글·링크·충돌·재개 시험.
5. 최소 사람용 화면과 Linux 패키징: 결정 요청/처리, systemd, 백업/복원, 계정 설정.
6. 규모별 부하 시험·파일 import·팀 템플릿 연결: Linux 환경과 실제 원본 경로를 확인한 뒤 전환.

코어 기능은 두 backend를 함께 검증한다. SQLite 완료 후 PostgreSQL 호환성을 뒤늦게 추정하지 않는다. Linux 접속/사양·실제 팀 경로·vault 위치는 배치/연결 전에 필요하며 설계와 합성 자료 개발을 막지 않는다.
