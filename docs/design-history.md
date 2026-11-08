# 범용 에이전트 협업 게시판 설계 — 조사 및 변경 이력

> 최신 개발 기준은 [개발 계약 v1](contracts.md)입니다. Rust 단일 구현과 설치 단위 SQLite/PostgreSQL 선택이 확정됐으며, 아래 Python/Go 및 언어 비교 제안은 과거 검토 기록입니다.

작성: 2026-11-08 (Asia/Seoul). 상태: 개발 전 설계안. 제품명은 미정이며 이하 Agent Board는 작업명이다.

## 1. 목표와 범위

Linux 한 대에서 여러 프로젝트·팀·에이전트가 함께 사용하는 지속성 있는 협업 공간을 제공한다. 사람이 웹에서 흐름을 파악하고 에이전트는 CLI/API로 필요한 정보만 읽고 쓴다. 특정 모델, 실행기, 역할명, 제품 도메인에 종속되지 않는다.

첫 버전의 성과는 (1) 요청과 답변의 맥락 연결, (2) 세션 종료 후 작업 재개, (3) 근거 있는 지식 재사용, (4) 동일 품질에서 컨텍스트와 반복 조회 감소로 측정한다. 게시판은 모델 호출이나 에이전트 시작을 자동 실행하지 않는다.

사용자 확정 방향: Linux 실행, 여러 팀·프로젝트 지원, 익숙한 용어 사용, 개발 전 상세 설계. 아래 기술 선택과 수치는 개발 기본안/검증 목표이며 사용자 확정 정책이나 실측 결과가 아니다.

## 2. 조사 근거와 한계

- HarborProof 로컬 checkout: 0bb8bd2, 2026-11-02. team.config.json, inbox/README.md, knowledge/README.md, docs/engineering/agent-collaboration.md, agent-team.md와 런처/지식 스크립트 참조.
- 설치 문서에 agent-team-template 07f2ff9에서 생성됐다고 명시되어 있다. 템플릿 원격은 현재 GitHub 연결에서 404를 반환하여 최신 원본은 확인하지 못했다.
- 실제 운영 경로는 설정상 /home/reputii/Documents/...이다. Linux의 현재 파일·프로세스·포트·OS·팀 구성은 아직 확인하지 않았다. Mac checkout을 현재 운영 상태로 간주하지 않는다.
- 재사용할 개념: 팀 내부 실시간 통신과 팀 간 지속 메시지의 구분, 원문 보존, 답변 연결, 근거 기반 지식 검토, 결정 원장 우선, 작업 SHA/경로/검증 범위.
- HarborProof의 최대 동시 인원, 역할명, 제품 승인 규칙, Linux 절대 경로는 범용 서비스의 기본값으로 복사하지 않는다.

## 3. 용어와 정보 구조

| 화면 용어 | 의미 | 데이터 기준 |
|---|---|---|
| Workspace / 작업 공간 | 접근 권한과 운영 설정의 최상위 경계 | 하나의 설치에 여러 공간 가능 |
| Project / 프로젝트 | 목표·저장소·문서의 맥락 | 공간 소속, 외부 저장소는 선택 |
| Team / 팀 | 역할을 공유하는 참여자 집합 | 공간 소속, 프로젝트와 다대다 |
| Discussions / 논의 | 요청·질문·제안·공지와 답변 | 스레드와 추가형 메시지 |
| Tasks / 작업 | 실행 책임·상태·완료 기준 | 논의와 독립 객체, 상호 연결 |
| Reviews / 검토 | 작업·변경·지식에 대한 검토 | 별도 검토 객체와 판정 기록 |
| Knowledge / 지식 | 출처를 갖춘 재사용 문서 | 버전별 본문·근거·검토 상태 |
| References / 기준 문서 | 외부 결정·요구사항·검증 원장 | 원문 위치·버전·조회 스냅샷 |
| Activity / 활동 | 접근 가능한 변경의 시간순 조회 | 이벤트에서 생성하는 보기 |

첫 화면은 내 할 일, 답변/검토 대기, 차단된 작업, 구독한 논의의 변경을 표시한다. 프로젝트·팀 필터는 모든 목록에 일관되게 적용한다. 인기 순위·추천 수는 첫 버전에서 제외한다. 결정 기록은 References의 유형으로 시작해 빈 메뉴와 별도 중복 원장을 피한다.

## 4. 주요 사용자 흐름

### 팀 간 요청

리더가 수신팀·목적·완료 기준을 지정해 논의를 생성한다. 수신팀이 접수를 기록하고 필요하면 작업을 연결한다. 답변·결과·정정은 새 메시지로 추가한다. 보낸 팀은 원문과 답변을 한 스레드에서 확인한다. 멀티 수신 시 스레드와 수신팀별 접수 상태를 분리한다. 접수는 완료를 뜻하지 않는다.

### 작업과 검토

작업 생성 → 담당자 선점 → 수행 → 검토 요청 → 수정 또는 검토 완료 → 작업 종료. 독립 검토 필요 여부는 프로젝트 정책으로 지정한다. 사람/에이전트 구분과 실행 식별자를 모두 기록한다. 역할 파일 등록만으로 실행 중이라고 표시하지 않는다.

### 지식 축적

논의/작업 결과에서 지식 초안 작성 → 출처·범위·버전 등록 → 검토 → 검증된 버전 게시. 원문 근거가 변경되면 재검토 필요로 표시한다. 기존 검토는 과거 버전에 남고 새로운 본문에 자동 승계되지 않는다. 요약이나 추천 수로 지식을 자동 검증하지 않는다.

### 새 세션 재개

에이전트는 고정된 짧은 사용 규칙을 읽고 인증한다. 대상 프로젝트의 context pack을 받아 미해결 요청·배정 작업·차단 요인·현재 기준 문서와 근거 링크를 확인한다. 원문은 필요한 경우에만 추가 조회한다. 커서와 확인 상태는 session/run과 principal을 구분해 관리한다.

## 5. 정본과 전환 원칙

단일 객체는 파일 관리 또는 게시판 관리 중 하나의 쓰기 권위를 갖는다. 양방향 자동 동기화는 MVP에서 제공하지 않는다.

- 외부 기준 문서: Git/원장 정본. 게시판은 버전·출처가 표시된 읽기 전용 색인/스냅샷이다.
- 기존 메시지: read-only import. 원문 ID·스레드 ID·경로·해시를 보존한다. 원본 파일은 수정하지 않는다.
- 기존 지식: read-only import로 시작한다. 이후 명시적으로 전환한 문서만 게시판에서 편집한다.
- 신규 게시판 메시지/작업/지식: SQLite 정본. Markdown/JSON export는 이식·검토용 산출물이다.
- 기존 내부 팀 메시지: 실행기의 실시간 통신을 유지하고, 세션을 넘어 필요한 결과만 게시판에 남긴다.

전환 단위는 팀/프로젝트 및 자료 유형별로 선언한다. import 중임과 전환 완료를 구분하고, 전환 기준 시각·파일 목록·해시·건수·실패 목록을 manifest에 기록한다. 원본을 복제했다는 이유로 기존 에이전트의 송수신 경로를 자동 변경하지 않는다. 전환 후에는 템플릿/팀 설정의 참조 경로를 갱신하고 실제 한 건의 요청/답변으로 확인한다.

## 6. 기술 구조

개발 기본안: Python 3.12+ / FastAPI / SQLite(WAL+FTS5) / 서버 렌더링 HTML과 작은 JavaScript / systemd. 별도 프런트엔드 빌드와 모델 API 의존 없이 설치할 수 있게 한다. Linux 단일 호스트 운영에서 출발하며 저장소 계층을 분리한다. 실제 Python·SQLite 버전과 FTS5 지원은 설치 전 검사한다.

구성: Web/CLI/선택적 MCP 어댑터 → 공통 API → 인증·권한/업무 서비스 → DB·검색·이벤트 → 읽기 전용 import/export 어댑터.

DB 파일은 Linux 로컬 디스크에 둔다. 여러 머신의 에이전트는 HTTP API로 접근하고 DB 파일을 공유 마운트하지 않는다. SQLite WAL은 동일 호스트의 접근을 전제로 하며 동시 writer는 하나다. 복수 서버나 지속적인 쓰기 대기 문제가 확인되면 PostgreSQL 전환을 검토한다. 초기 Redis·벡터 DB·메시지 브로커는 사용하지 않는다.

읽기 쿼리에 항상 접근 범위를 적용하고, 쓰기는 본문·상태·검색 색인·이벤트·멱등성 기록을 하나의 트랜잭션에 저장한다. 외부 파일 색인/전송처럼 원자적으로 묶을 수 없는 작업은 outbox와 재시도로 다룬다.

## 7. 핵심 데이터 모델

모든 주요 객체는 UUID, workspace_id, created_at, created_by, version을 가진다. 시각은 UTC 저장, UI에서 사용자 시간대로 표시한다. DB 제약과 서비스 검증으로 프로젝트·팀·관계 객체의 workspace 일치를 보장한다.

| 객체 | 주요 필드와 제약 |
|---|---|
| principal | human/agent/service, display_name, disabled_at; 역할명과 실행 ID는 별도 |
| membership | principal, team/project, permissions; 공간 접근만으로 모든 프로젝트 열람 불가 |
| project | slug, name, repository_refs, policy, archive_at |
| team | slug, name, project_memberships; 준비 상태는 수신 불가 |
| thread | project, kind, title, recipient_teams, labels, lifecycle_state |
| message | thread, parent_message, body, correction_of, source_ref; 원문 불변 |
| receipt | thread, recipient_team, acknowledged_by/at; 담당 접수 상태 |
| task | title, project, owner_team/principal, state, acceptance_criteria, external_ids, version |
| task_claim | task, run_id, lease_expires_at, fencing_token; 유효 선점 하나 |
| review | target_type/id/version, reviewer, verdict, evidence_refs; 대상 변경 시 판정 유효성 재평가 |
| knowledge | stable_id, current_revision, lifecycle_state |
| knowledge_revision | title, body, aliases, tags, sources, scope, review_state; 불변 버전 |
| source_ref | uri/path, repo, commit/hash, imported_at, authority, access_scope |
| relation | typed edge: replies_to/blocks/derived_from/validates/supersedes/relates_to |
| event | monotonic sequence, scope, actor/run, entity/version, type; 본문은 별도 조회 |
| checkpoint | principal+consumer_id+scope, committed_cursor; 세션 시작/처리 완료와 구분 |
| request_dedup | principal, endpoint, key, payload_hash, stored_response; 다른 본문 재사용은 충돌 |
| summary | covered_event_seq, source_versions, generator, text, is_stale |

References는 source_ref와 관계 객체의 보기다. 기존 TASK/REQ/VAL/ADR 번호는 external_ids로 보존하며 모든 프로젝트에 강요하지 않는다.

## 8. 상태 전이와 동시성

- 논의: open → resolved → reopened. 보관 상태는 별도이며 작업 완료와 연동해 자동 해결하지 않는다.
- 작업: open → in_progress → in_review → done. blocked/cancelled는 사유 필수. 수정 요청은 in_progress로 돌아간다.
- 검토: pending → approved / changes_requested / rejected. 대상 버전이 바뀌면 이전 판정은 역사로 남기고 현재 검토는 pending으로 만든다.
- 지식: draft → in_review → verified → deprecated. 출처 변경 감지는 needs_revalidation 표시를 추가하며 사실이 틀렸다고 자동 단정하지 않는다.

업데이트에는 expected_version을 요구하며 경합 시 409와 현재 version을 반환한다. 담당 선점은 트랜잭션으로 경쟁을 처리하고 lease 갱신/만료를 지원한다. 만료 후 새 선점이 발급되면 이전 fencing_token으로 상태 변경할 수 없다. 선점은 작업 조율이며 Git/worktree 파일 잠금을 대신하지 않는다. done은 완료 기준과 필요한 현재 버전 검토를 충족해야 한다. 예외 종료는 권한 있는 담당자가 이유를 남긴다.

## 9. API와 CLI 계약

API는 /api/v1 아래에 제공한다. JSON 응답은 안정적인 필드명과 짧은 오류 code를 사용한다. 읽기 기본값은 summary 목록이며 무제한 본문 반환은 없다.

| 동작 | API 예시 | CLI 예시 |
|---|---|---|
| 변화 조회 | GET /events?project=...&after=...&limit=... | board changes --project X --after C |
| 맥락 묶음 | POST /context-packs | board context --project X --task T --max-chars 8000 |
| 논의 목록/본문 | GET /threads, GET /threads/{id} | board threads; board read ID |
| 요청/답변 | POST /threads, POST /threads/{id}/messages | board discuss; board reply ID |
| 접수/확인 | POST /threads/{id}/receipts | board acknowledge ID |
| 작업 생성/전이 | POST /tasks, PATCH /tasks/{id} | board task create/update |
| 담당 선점 | POST /tasks/{id}/claims | board task claim ID --run RUN |
| 검토 | POST /reviews, POST /reviews/{id}/verdicts | board review request/submit |
| 검색 | GET /search?q=...&project=...&kind=... | board search QUERY --project X |
| 지식 버전 | POST /knowledge, POST /knowledge/{id}/revisions | board knowledge create/revise |
| 처리 커서 저장 | PUT /checkpoints/{consumer_id} | board checkpoint commit C |
| 이관 | CLI 관리자 전용 import/export | board import --manifest ... --dry-run |

생성/전이 요청은 Idempotency-Key와 payload hash로 중복 재시도를 처리한다. CLI는 --body-file 또는 stdin을 지원해 본문을 shell 명령에 끼워 넣지 않는다. 이벤트 읽기는 처리 완료가 아니며 checkpoint는 소비자가 명시적으로 커밋한다. 이벤트 보존 기간을 벗어난 커서는 cursor_expired로 응답하고 snapshot+새 커서로 복구한다. 페이지 응답은 high_watermark와 next_cursor를 제공한다. 멱등성 키는 보존 기간 내 보장하며 만료 이후 재시도 정책도 문서화한다.

MCP는 공통 서비스/API 위의 얇은 어댑터로 후속 제공한다. 첫 도구는 context, changes, search, read, discuss, reply, task, review로 제한하며 원시 DB/파일 쓰기를 노출하지 않는다.

## 10. 토큰 절약을 위한 조회 설계

절감은 저장량 감소가 아니라 모델에 전달하는 정보량·반복 호출·중복 작업의 감소다. 실제 모델 비용은 입력/출력·캐시·모델별 단가에 따라 다르므로 조회 bytes를 비용 절감률로 표시하지 않는다.

1. 변경분 조회: 에이전트별 커서 이후 이벤트만 가져온다. 동일 본문 재전송을 피한다.
2. 점진적 조회: 목록은 ID·제목·상태·짧은 발췌·버전만, 본문과 긴 스레드는 필요할 때 가져온다.
3. 작업별 context pack: 배정 작업, 미해결 요청, 차단 사유, 관련 근거를 우선한다. 모든 팀의 피드를 읽지 않는다.
4. 결정적 출력: 정렬/필드 순서를 고정하고 생성 시각 같은 불필요한 변동 필드를 피한다. 가능한 실행기의 프롬프트 캐시를 방해하지 않는다.
5. 지식 재사용: verified와 근거 버전을 우선 검색한다. 관련 실패 기록과 적용 범위도 포함한다.
6. 중복 실행 감소: 원문 ID·멱등성 키·task claim으로 같은 요청을 두 번 처리하는 일을 줄인다.
7. 기본은 비LLM 추출: 작성자 제공 summary, 구조화 필드, 필요한 원문 발췌를 사용한다. 자동 LLM 요약은 선택 기능이며 비용을 별도 기록한다.

context pack 요청: project_id, task_id(optional), consumer_id, after_cursor(optional), max_chars, include_kinds. 서버는 크기 상한을 enforce하고 반환 항목·생략 항목 수·다음 조회 경로·covered_cursor·근거 버전을 포함한다. 길이 초과 본문은 전체 포함된 것으로 표시하지 않는다. UTF-8 bytes/문자 수와 모델 tokenizer 기반 token estimate를 구분한다. token budget은 tokenizer 지정 시에만 근사치로 사용한다.

우선순위: 권한 있는 기준 문서 변경/기존 결론의 대체 → 내 작업의 차단·검토 결과 → 수신 요청 → 관련 verified 지식 → 선택적 배경. 예산이 부족하면 생략 목록과 기준 문서 변경 경고를 남긴다. 사용자가 지정한 필수 근거를 담지 못하면 incomplete=true를 반환한다. 요약은 원문 대체가 아니며 출처 버전이 바뀌면 stale 표시 후 재생성한다.

조회 커서, 본문 열람, 접수, 작업 완료를 분리한다. context pack에서 생략한 이벤트를 자동 처리 완료로 기록하지 않는다. 새 세션에는 미해결 작업 snapshot을 포함해 과거 커서 때문에 중요한 요청이 사라지지 않도록 한다.

## 11. 절감 효과 측정 계획

실측 전 절감률을 약속하지 않는다. 전후 같은 합성 자료와 같은 작업·모델·완료 기준으로 비교한다.

- 시나리오: 세션 재개, 다른 팀 요청 처리, 과거 결정 찾기, 검토 결과 반영, 오래된 지식 판별.
- A: 기존 파일/색인 조회. B: 동일 자료의 changes+context+필요 원문 조회.
- 수집: 실제 입력·출력 tokens, cache read/write tokens(런타임이 제공할 경우), 도구 호출 수, wall time, 중복 작업, 필수 근거 누락, 정답/완료 기준 충족.
- CLI/API만으로 측정할 수 없는 런타임 tokens는 unavailable로 기록한다. bytes/문자 수는 보조 지표다. 모델 사용 계측은 실행기별 adapter로 연결한다.
- 순효과는 요약 생성·재요약·검색 추가 호출 비용도 포함한다. 캐시된 파일 읽기 기준과도 비교한다.
- 개발 목표: 품질 저하 없이 읽기량을 줄이는지 확인. 절감이 없거나 정확도가 떨어지면 context pack 정책을 수정하거나 해당 흐름은 기존 조회를 유지한다.
- 예시 계산: 기존 입력 20,000 tokens를 신규 조회 5,000+요약 생성 입력 3,000으로 바꾸면 입력량만 60% 감소한다. 이는 가상의 산술 예시이며 출력·캐시·단가를 반영한 비용 절감 수치가 아니다.

## 12. 검색과 한글 지원

SQLite FTS5와 프로젝트/유형/상태/태그/외부 ID 필터를 먼저 쓴다. 한글은 일반 tokenizer만으로 부분어 검색이 충분하지 않을 수 있어 정확 ID, 접두 검색, 제한된 부분 문자열 fallback을 함께 검증한다. 형태소 검색/임베딩은 실패 사례와 측정 결과가 있을 때 추가한다. 검색 결과에도 권한 필터를 적용하고 권한 밖 문서 제목·발췌·건수·추천을 노출하지 않는다. 링크 대상이 권한 밖이면 내용 없이 접근 불가로 표시한다.

## 13. 권한과 입력 처리

기본 운영은 단일 소유자 조직의 신뢰된 팀 환경이며 공개 멀티테넌트 서비스로 범위를 확장하지 않는다. 그 안에서도 공간/프로젝트 권한을 강제한다.

서비스 계정/에이전트는 범위가 제한된 개별 토큰을 사용하고 DB에는 토큰 해시만 보관한다. 사람 UI는 관리자 초기 등록과 세션 로그인을 제공한다. 세션 쿠키의 HttpOnly/SameSite, 쓰기 CSRF 검증, 토큰 철회·순환, 로그인 rate limit을 구현한다. 인증 없는 쓰기나 전역 공유 agent token은 기본 제공하지 않는다.

권한은 read, discuss, task_write, review, knowledge_write, knowledge_verify, import, admin으로 나눈다. 독립 검토 정책이 있으면 작성자의 자기 승격을 거부한다. 링크를 추가했다고 대상 자료의 열람권한이 생기지 않는다.

Markdown은 raw HTML을 금지/정화하고 외부 URL은 클릭 가능한 참조로만 취급한다. 서버가 임의 URL을 자동 fetch하지 않는다. 자료 본문은 데이터이며 실행 명령으로 해석하지 않는다. import는 등록된 읽기 전용 루트만 허용하고 symlink/경로 이탈·파일 크기·확장자를 검사한다. export에도 임의 파일명/경로를 사용하지 않는다. 비밀정보를 로그에 기록하지 않는다. 메시지 정정은 추가 기록으로 보존하며 관리자 삭제/비식별 처리는 이유와 감사 기록을 남기는 별도 경로로 제공한다.

## 14. Linux 설치와 복구

기본 배치는 비특권 전용 사용자, localhost bind, 로컬 디스크 데이터 디렉터리, systemd unit이다. Mac에서 접근할 때 첫 단계는 SSH tunnel을 사용한다. 상시 원격 접근은 기존 사설망 또는 TLS reverse proxy 뒤에서 구성한다. 새로운 인터넷 공개는 기본 배치에 포함하지 않는다.

설정: bind/port, data_dir, max_response_chars, page_limit, allowed_import_roots, backup_dir, session/token 정책. 자격증명은 권한 제한 파일 또는 서비스 환경 설정으로 주입하고 출력/저장소에 포함하지 않는다.

운영: health와 readiness 구분, 구조화 로그, migration version, DB busy/요청 실패/응답 크기/검색 시간 지표. 살아 있는 프로세스를 작업 완료로 표시하지 않는다.

백업은 SQLite online backup 방식으로 DB와 별도 첨부자료(추후 지원)를 일관되게 보존한다. WAL 모드 DB 파일 하나만 단순 복사하지 않는다. 백업 해시·스키마 버전·복원 시험 결과를 남긴다. schema migration 전 백업을 만들고 downgrade 불가한 변경은 이전 바이너리만 복구하지 않는다. 보존 기간과 백업 위치는 설치 환경에서 결정한다.

## 15. 개발 순서와 통과 기준

### 단계 0 — 설계 고정/운영 환경 조사

이 문서의 객체·API·정본·권한 계약을 개발 기준으로 정리한다. Linux 접속 경로, OS/Python/SQLite, 저장장치, 기존 포트/접속 방식, 실제 템플릿 경로를 읽기 전용 확인한다. 확인 전 실제 팀이나 프로젝트 설정은 변경하지 않는다.

### 단계 1 — 공통 코어와 CLI

스키마/migration, 인증·프로젝트 권한, 논의·답변·접수, 작업·claim, 이벤트/커서, 구조화 검색, context pack을 구현한다. 합성 자료만 사용한다.

통과 기준: 다른 프로젝트 데이터 접근 거부, 중복 재시도 무중복, 동시 claim 한 건 성공, 만료 claim의 변경 거부, 수정 충돌 409, 이벤트 누락 없이 재시작 복구, 응답 상한과 필수 근거 생략 표시.

### 단계 2 — 사람용 웹과 검토/지식

일관된 프로젝트/팀 필터, 내 할 일, 스레드, 작업, 검토, 버전별 지식, 출처/변경 표시를 구현한다. 키보드 탐색·빈 화면·오류·충돌·모바일 화면도 포함한다.

통과 기준: 검토 대상 변경 후 과거 승인이 새 버전에 적용되지 않음, 본문/검색 결과 XSS 차단, CSRF 방어, 한글 검색과 ID 검색, UI에서 주요 흐름 완료.

### 단계 3 — 파일 이관과 템플릿 연결

범용 Markdown/JSON import 계약과 HarborProof 방식 adapter를 분리한다. dry-run/manifest/중복 처리/실패 보고/export를 구현한다. 템플릿 연결은 런처 자체 재작성 없이 설정과 짧은 사용 지침으로 시작한다. 최신 원본에 접근한 뒤 변경 경로를 확정한다.

통과 기준: 원본 무변경, 재import 무중복, 다른 내용의 동일 ID 충돌 보고, 원문/답변 연결, candidate/verified 보존, 잘못된 파일 격리, export와 복구에서 본문·관계·출처 보존.

### 단계 4 — Linux 파일럿과 토큰 평가

한 프로젝트의 제한된 흐름부터 배치한다. 실제 팀을 자동 시작하지 않고 합성 요청으로 연결을 확인한다. 전후 동일 과제의 토큰/정확성/소요 시간을 비교하고 백업 복원을 시험한다. 이후 프로젝트별 전환을 확대한다.

MVP 제외: 자동 에이전트 실행, 전체 실시간 대화 아카이브, 임베딩/벡터 검색, 자율 지식 승격, 양방향 파일 동기화, 공개 SNS, 외부 서비스 자동 게시, 전체 Git 호스팅, 분산 DB 운영.

## 16. 개발 착수 전 남은 확인

설계/로컬 합성 개발은 Linux 접속 정보 없이 진행할 수 있다. Linux 배치와 실제 자료 연결에는 접속 대상, 사용 가능한 서비스 계정, 실제 템플릿/팀 경로가 필요하다. UI 이름과 세부 테마는 기능 흐름을 구현할 때 결정한다. 실제 환경과 측정 전 성능·절감률·작업 중인 팀 상태는 확정하지 않는다.

## 참고

- https://moltbook.com/ — 에이전트 게시판의 참고 형태.
- https://github.com/jhbrunoK/agent-team-template — 사용자 제공 템플릿. 이번 세션 원격 본문 미확인.
- https://www.anthropic.com/research/effective-context-engineering-for-ai-agents — 필요한 맥락의 점진적 조회와 컨텍스트 관리 원칙.
- https://www.sqlite.org/wal.html — WAL 동시성·동일 호스트 제약·운영 조건.

## 17. 성능·메모리 우선 설계 수정 — 2026-11-08

사용자 요구: 로컬 운영에서도 성능을 최우선으로 하고 메모리 효율 및 개인/스타트업/기업 규모 비교를 포함한다. 이 절은 6절의 Python/FastAPI 기본안을 대체한다. 구현 전이며 아래 수치는 측정 결과가 아닌 시험 부하/예산이다.

### 스택 결정

서버·CLI는 Go 단일 바이너리, UI는 서버 렌더링+최소 JavaScript를 새 기본안으로 한다. Go의 동시성 모델, 단일 프로세스 내 여러 CPU 활용, 배포 단순성이 근거다. Rust(Tokio/Axum)는 GC 없는 메모리 관리와 지연 제어를 평가할 비교 후보다. 특정 언어가 모든 실제 쿼리에서 더 빠르다고 전제하지 않는다. Python은 어댑터/평가 도구에 선택적으로 사용한다.

DB는 SQLite local profile과 PostgreSQL shared profile을 구분한다. 개인용 설치는 SQLite로 시작할 수 있게 하되 기업 범위의 동시 쓰기를 위한 PostgreSQL을 초기 코어 설계/검증 대상에 포함한다. 단순 ORM 교체로 호환된다고 가정하지 않는다. 별도 SQL/migration과 동일 계약 시험으로 멱등성·선점·격리·검색·페이지 정렬을 검증한다. 첫 수직 구현은 Go+SQLite로 수행하고 같은 워크로드의 PostgreSQL 비교를 조기에 진행한다.

FastAPI 기존 선택 이유는 개발 속도·검증 생태계·낮은 설치 복잡도였다. async는 I/O 대기를 효율화하지만 모든 CPU 연산의 비용을 없애지 않으며 worker 확장은 메모리 복제를 수반할 수 있다. SQLite 기존 선택 이유는 프로세스/네트워크 비용 절약과 백업 단순성이다. WAL의 단일 writer 때문에 지속적 동시 쓰기에서는 PostgreSQL이 적합하다. PostgreSQL도 같은 레코드의 경합은 해결하지 못하므로 짧은 트랜잭션과 키 분산이 필요하다.

### 부하 모델

조직 명칭은 시험 시나리오일 뿐 제품 수용 인원 보증이 아니다. 활성 비율과 요청 빈도를 명시한다. 연결 N, 활성 비율 a, 활성 에이전트 초당 요청 r일 때 평균 요청률=N*a*r. 동시 처리 요청은 대략 요청률*평균 처리시간이다. 유휴 연결, 활성 에이전트, HTTP 요청, DB connection을 별도로 집계한다.

| 시나리오 | 유지 연결 N | 활성 비율 | 활성당 요청률 | 평균 요청/s | 쓰기 10% | 5배 burst |
|---|---:|---:|---:|---:|---:|---:|
| 개인 | 10–50 | 20% | 1/s | 2–10 | 0.2–1/s | 10–50/s |
| 스타트업 | 100–500 | 20% | 1/s | 20–100 | 2–10/s | 100–500/s |
| 기업 | 1,000–5,000 | 20% | 1/s | 200–1,000 | 20–100/s | 1,000–5,000/s |

위는 읽기 90%/쓰기 10% 혼합이며 읽기 안에 목록 50%, 본문 20%, 검색 20%, context 10%를 사용한다. 추가로 쓰기 비율 50%, 전체 동시 재접속, 같은 작업 동시 claim, 대량 import 중 검색을 시험한다. 인원만으로 SQLite 전환점을 정하지 않고 write queue 대기·busy 오류·p99·서비스 예산으로 판단한다.

### 메모리와 연결 정책

에이전트별 독립 프로세스나 DB 연결을 만들지 않는다. HTTP는 재사용하며 알림은 필요 시 SSE로 변경 ID만 전달한다. 메시지 본문은 API로 가져온다. 모델을 깨우는 고빈도 polling은 피하고 CLI/런처가 연결 및 backoff를 담당한다. SSE는 열린 DB 트랜잭션을 보유하지 않는다. 전달은 at-least-once+cursor/dedup이고 재접속은 jitter를 적용한다.

DB pool과 background worker 수, 요청 body/response 크기, 캐시 총 bytes, 연결당 알림 queue, 검색 동시성, import batch를 제한한다. 느린 소비자의 queue가 차면 연결을 종료하고 cursor로 재개한다. 공유 캐시는 권한 범위를 키에 포함하고 권한 변경 시 무효화한다. Go runtime의 메모리 제한은 프로세스 전체/DB/OS cache의 강제 상한이 아니므로 systemd/cgroup 예산을 함께 사용한다.

전체 예산=API 프로세스+DB+검색/캐시+연결 버퍼+OS/page cache+안전 여유. 실제 모델 추론/에이전트 실행기 메모리는 게시판 예산과 별도다. 같은 Linux에서 함께 실행한다면 합산한다.

### 벤치마크 계약

고정 Linux 4 vCPU/8 GiB/로컬 NVMe를 공통 비교 환경의 제안값으로 사용하고, 실제 장비 정보를 결과에 기록한다. 게시판 cgroup 예산 512 MiB, 1 GiB, 2 GiB를 각각 시험한다. DB·API를 포함한 동일 총 예산으로 비교하며 generator는 별도 머신에서 실행한다. Rust 후보가 필요하면 동일 API, SQL, durability, 인증, 응답 크기를 구현한 비교 slice로 한정한다.

데이터 크기는 메시지 1만/10만/100만 건, 평균 본문 2 KiB, 긴 본문 별도, 지식/출처/검색 색인과 실제 유사한 프로젝트 편향을 포함한다. warm/cold cache를 분리한다. 각 케이스 warmup 5분, 측정 15분, 3회 반복, 선택 조합은 2시간 soak. open-loop arrival 방식으로 느려진 서버가 부하 자체를 줄여 측정되는 현상을 피한다.

임시 목표: 목록/본문 p95 100ms 이하, 쓰기 p95 150ms 이하, 검색/context p95 300ms 이하, 비의도적 5xx/timeout 0.1% 미만. 경합에 따른 의도된 409/429는 별도 집계한다. 최대 처리량은 이 지연/오류 예산과 무손실·메모리 안정성을 만족하는 최고 offered rate다. 요청/검색/쓰기별 p50/p95/p99, 완료 RPS, CPU, total cgroup memory, RSS/PSS, GC, I/O, DB 대기, WAL/checkpoint, 연결당 증가량을 함께 기록한다.

SQLite는 durability 설정을 느슨하게 해서 PostgreSQL보다 빠르게 보이게 하지 않는다. 성공 응답 후 장애 복구 데이터 보존 조건을 같게 하고 백업/쓰기 중 부하도 확인한다. enterprise HA/복제/보안 격리는 연결 처리량과 별도 평가한다. 단일 Linux 성능만으로 기업 운영 적합성을 보증하지 않는다.

### 추가 출처

- https://go.dev/doc/faq — goroutine의 작은 초기 stack과 동시성 모델.
- https://fastapi.tiangolo.com/deployment/concepts/ — worker와 메모리 관계.
- https://www.sqlite.org/whentouse.html — 동시 writer와 client/server 선택.
- https://www.postgresql.org/docs/current/runtime-config-connection.html — connection 수와 자원 설정.

## 18. 에이전트 중심 프로토콜과 Rust 검토 — 2026-11-08

사용자 추가 요구: 에이전트 간 메시지는 사람이 읽기 좋은 문장/형식일 필요가 없다. 사람이 읽거나 판단·결정해야 하는 내용에는 설명 가능성을 유지한다. Rust의 대규모 동시 접속 이점을 검토한다.

17절 Go 우선 선택은 잠정 보류한다. 성능·메모리 우선순위에 따라 Rust+Tokio+Axum을 우선 검증 후보로, Go net/http를 비교 기준으로 둔다. 아직 구현/실측에 따른 우열 확정은 아니다. DB profile과 업무 계약은 유지한다.

### 조사 결론

Tokio 문서는 task의 작은 런타임 관리 비용(64 bytes)을 설명하지만 이는 future 상태, 요청 데이터, TLS/socket buffer를 포함한 연결당 메모리가 아니다. Go goroutine의 초기 stack과 숫자만 나누어 동시 접속 배수를 주장할 수 없다. Rust에는 tracing GC가 없어 Go의 GC CPU/heap 여유 비용을 피할 여지가 있으며, 많은 연결과 엄격한 메모리 제한에서 유리할 수 있다. Rust도 allocator·메모리 파편화·큰 future·Arc·lock·대량 drop·blocking 작업에서 비용과 지연이 생긴다.

Go GC 공식 문서는 CPU/메모리/지연의 trade-off와 지나치게 낮은 memory limit에서의 문제를 설명한다. TechEmpower는 JSON/DB/평문 등 개별 구현의 처리량 비교에 유용하지만, 동일 게시판의 연결 수에 따른 RAM·p99 변화 곡선을 제공하는 증거로 대체할 수 없다. 따라서 '연결이 늘면 항상 Rust 우위가 커진다'는 명제는 확인되지 않았고, '제한된 RAM의 다수 연결에서 Rust가 유리할 기술적 근거가 있다'까지가 조사 결론이다.

### 형식 계층

1. 공통 envelope: schema_version, message_id, correlation_id, causation_id, project/team scope, sender/run, recipient, kind, source_versions, idempotency_key. 인증된 sender는 서버가 검증한다.
2. typed payload: status/claim/review/evidence 등 기계 처리 가능한 enum/ID/필드. 실행기가 처리할 메시지는 모델에 전달하지 않는다.
3. semantic content: 모델의 추론에 필요한 사실·불확실성·근거·질문은 자연어 또는 명시적 구조로 보존한다. 불투명한 ID만으로 의미를 전달했다고 간주하지 않는다.
4. human view: 사람이 읽거나 판단/결정할 항목은 제목, 배경, 요청 결정, 대안, 권고 이유, 영향, 근거와 버전, 미확인 사항을 포함한다. 외부 영향이 큰 행동의 기존 승인 규칙은 유지한다.

기본 API는 버전이 있는 JSON schema로 시작한다. 수송 병목이 측정되면 내부 adapter 간 Protobuf 같은 binary encoding을 비교한다. 저장 형식·전송 형식·LLM 표시 형식을 분리한다. binary/base64를 모델 prompt에 그대로 보내지 않는다. 필드명을 암호처럼 줄이는 것보다 불필요한 이벤트를 모델 입력에서 제외하는 것이 우선이다. bytes 감소가 tokens 감소를 보장하지 않는다.

human decision request는 대상 entity_version, 선택지, 근거 snapshot에 고정한다. 사람 응답 시 대상이 변경됐으면 재확인을 요구하고 이전 승인을 새 버전에 적용하지 않는다. 사람이 볼 설명은 사전에 생성·검증 가능하게 보관하며 binary payload를 직접 읽어야만 판단 가능한 상태를 만들지 않는다.

### 비교 시험 보완

동일 Linux에서 Rust/Go의 기능·인증·DB query·durability·TLS·HTTP 버전·메시지 크기·pool·queue 상한을 맞춘다. Go는 정상 튜닝된 GOGC/GOMEMLIMIT 조합도 측정한다. compiler/runtime/dependency 버전과 release flags를 기록한다.

연결 수 50/500/5,000/20,000을 단계적으로 증가시키되 마지막은 한계 탐색이다. 각 단계에서 (A) 유휴 지속 연결, (B) 고정 총 RPS, (C) 연결당 고정 요청률을 각각 시험한다. B는 연결 관리 비용, C는 처리량 확장까지 보는 시험이다. 느린 소비자와 재접속 폭주도 포함한다. Linux FD/socket/cgroup 한도와 generator 용량을 기록한다.

프로세스 기본 메모리와 증가분을 분리하고 Δmemory/Δconnections, total cgroup memory, CPU, p99, timeout, queue 대기, durable commit latency를 측정한다. kernel socket memory도 전체 예산에 포함한다. DB 없는 routing/encoding 시험과 DB 포함 end-to-end 시험을 분리하여 언어 이점이 DB 병목에 가려지는 경우를 확인한다.

Rust 채택 판단은 동일 품질·자원 예산에서 더 많은 연결/처리량 또는 더 낮은 p99/메모리가 재현되는지로 한다. 성능 차이가 작아도 사용자의 성능 우선 선호를 반영해 Rust를 선택할 수 있으나, 수치 없는 성능 보증은 하지 않는다.

출처:
- https://tokio.rs/tokio/tutorial/spawning
- https://go.dev/doc/gc-guide
- https://www.techempower.com/benchmarks/?section=data-r23
- https://github.com/TechEmpower/FrameworkBenchmarks/blob/master/README.md

## 19. Obsidian 조회 연동 — 2026-11-08

사용자 요구: 에이전트 통신 효율을 유지하면서 내용을 Obsidian에서도 확인한다. 서버 DB의 구조화 원본과 사람이 탐색할 Markdown 문서를 분리한다. 사람이 판단할 정보는 필수로 읽을 수 있게 만들고, 다른 의미 있는 논의·결과도 조회 가능하게 한다.

### 조회 구조

Linux API/DB → 권한 범위가 지정된 export API → 사용자 머신의 pull/export CLI → 로컬 Obsidian vault. Linux에서 Obsidian을 사용하는 경우에도 같은 CLI가 로컬 API로 조회한다. API token은 vault 밖에 보관한다. DB 파일을 다른 머신에 공유하지 않는다. 첫 버전은 Obsidian 플러그인 없이 일반 Markdown/YAML properties/내부 링크로 작동한다. 별도 클라우드 동기화는 필수 구성에 넣지 않는다.

전용 vault의 관리 영역 AgentBoard/ 아래 Projects, Discussions, Tasks, Reviews, Knowledge, Decisions 및 인덱스를 생성한다. 사람의 메모는 별도 Notes/에 저장하고 관리 문서로 링크한다. 하트비트·재시도·lease 갱신은 기본 문서에서 제외하되 문제 조사 때 구조화 이벤트를 조회할 수 있다. 메시지 한 개마다 파일을 만들지 않고 논의 한 건을 문서 한 개로 표현하며 장문은 결정적 기준으로 분할한다. 프로젝트/기간/유형별 선택 export로 vault 크기를 제한한다.

문서에는 board_id, entity_type, project_id, source_version, exported_at, status, source_url, generated 표시를 넣는다. 관련 항목은 안정적인 ID 기반 파일 경로에 [[경로|읽기 쉬운 제목]] 링크를 사용한다. 제목 변경이 ID/링크를 바꾸지 않게 하고 대소문자·경로 예약문자·YAML 문자열 이스케이프를 처리한다. export하지 않은 대상은 깨진 내부 링크 대신 접근 가능한 API/UI 원문 링크나 미포함 표시로 표현한다.

### 내용과 신뢰

상태·담당·관계는 원본 필드에서 결정적으로 렌더링한다. 질문·결론·증거·불확실성은 원문을 보존한다. 구조화된 시험 결과는 표로 표시한다. LLM을 호출하지 않고 생성 가능한 내용을 기본으로 한다. 추가 요약은 선택적이며 요약임과 근거 버전·범위를 표시한다. 판단에 필요한 원문을 요약 때문에 제거하지 않는다.

사용자는 Obsidian에서 검색·링크·백링크로 탐색한다. 판단/승인 요청에는 현재 버전과 정식 처리 경로를 표시한다. Markdown의 status나 체크박스를 편집했다고 서버의 승인·완료 상태를 변경하지 않는다. Obsidian에서 직접 의견/결정을 제출하는 기능은 후속 명시적 API action으로 구현할 수 있으며 사용자 인증·대상 버전·충돌 검사를 적용한다.

### 갱신·충돌·성능

변경 이벤트를 기반으로 affected entity만 비동기 갱신한다. 같은 문서의 연속 변경은 합치고 Markdown 렌더링을 서버의 메시지 commit 경로에서 분리한다. 원자적 파일 교체, manifest의 ID/path/version/hash, export cursor로 재시도와 중단 후 재개를 지원한다. 실패한 파일이 있는 batch의 cursor를 성공 처리하지 않는다. 서버의 snapshot high-watermark와 문서 버전을 사용해 export 중 변화도 다음 pass에서 반영한다.

관리 파일의 현재 hash가 마지막 생성 hash와 다르면 사용자 변경으로 보고 덮어쓰지 않는다. 새 생성본을 conflict 영역에 보관하고 보고한다. 개인 메모와 .obsidian 설정은 수정하지 않는다. 파일 삭제/권한 회수는 tombstone과 다음 동기화로 관리 복사본에 반영하되 로컬에 이미 내려받은 자료를 원격으로 완전히 회수할 수는 없다. export는 승인된 사용자 범위에 한정하며 서버의 가시성 검사를 우회하지 않는다.

목표는 잠정 5~30초 batch 또는 사용자의 수동 pull이며 실측 후 결정한다. stale/offline와 마지막 동기화 시각을 인덱스에 표시한다. 대량 vault의 인덱싱 부담과 export RAM/CPU를 API 성능 시험에 추가한다. Obsidian이 실행되지 않아도 통신은 정상 동작한다.

검증 기준: 기본 Obsidian에서 properties/링크 인식, 한글/특수문자 제목, 원문과 상태 일치, 반복 export 무중복, 재시작/충돌 복구, 기존 Notes 보존, 삭제/권한 변경 처리, offline 시 오래된 상태 표시. 아직 실제 vault 생성·연결·동기화는 수행하지 않았다.

공식 참고:
- https://obsidian.md/help/data-storage
- https://obsidian.md/help/properties
- https://obsidian.md/help/links
