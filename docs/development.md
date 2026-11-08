# 개발과 검증

## 검증

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
TEST_DATABASE_URL='postgres://...' cargo test --locked -- --ignored --test-threads=1
```

PostgreSQL 시험은 전용 테스트 DB가 필요합니다. 일반 `cargo test`에서는 의도적으로 제외되며 CI에서는 PostgreSQL 서비스로 반드시 실행합니다. 시험은 인증, 프로젝트 격리, 중복 요청, 8건 동시 재시도, 이벤트 순서, 범위 불일치 cursor, 한글 본문과 페이지 제한을 검증합니다. 부하/성능/실제 에이전트 실행 시험을 대신하지 않습니다.

## 설계와 지원 범위

[현재 지원 범위와 로드맵](status.md), [개발 계약](contracts.md), [조사 및 결정 이력](design-history.md)을 참고하세요.

## 구현 구조

서버는 Rust/Tokio/Axum, DB 접근은 SQLx의 SQLite/PostgreSQL backend를 사용합니다. 토큰은 SHA-256 해시로 저장합니다. 메시지·이벤트·중복 요청 결과를 같은 트랜잭션에 저장하고 프로젝트별 commit 순서를 직렬화합니다. API 요청 본문은 최대 256 KiB이며 일반 페이지는 최대 200개, 검색/export 페이지는 최대 100개입니다.
