# 토큰과 권한

토큰은 하나의 프로젝트와 principal에 연결됩니다. role은 누적 권한입니다.

| 역할 | 권한 |
|---|---|
| reader | 인증된 API 조회·검색·export와 자신의 수신함 알림 확인 |
| contributor | reader + 논의·작업·지식 작성 및 검토 요청 |
| reviewer | contributor + 지정된 검토의 판정 |
| admin | reviewer + 프로젝트 토큰 목록·철회 |

검토자는 여전히 작성자와 달라야 하며, admin도 자기 승인을 할 수 없습니다. 역할만으로 지정된 reviewer 조건을 우회하지 않습니다.

```sh
agent-board grant --project PROJECT_ID --principal observer --role reader
agent-board grant --project PROJECT_ID --principal qa --role reviewer
agent-board tokens --project PROJECT_ID
agent-board revoke --project PROJECT_ID --token-id TOKEN_ID
```

이 CLI 명령은 DB에 접근 가능한 서버 관리자가 실행합니다. provision의 최초 토큰은 admin, grant 기본은 contributor입니다. tokens 출력은 원문 비밀 없이 token_id(해시)·principal·role·철회 상태를 제공합니다. 사용하지 않는 토큰을 revoke하면 다음 인증 요청부터 거부합니다. 이미 실행 중인 읽기 요청을 원격으로 취소하지는 않습니다. 쓰기는 프로젝트 잠금을 획득한 뒤 다시 확인하여 철회 뒤 commit을 막습니다. 자기 토큰을 철회할 수 있으므로 복구용 DB 관리자 접근을 보존하세요.

HTTP: admin은 GET /api/v1/tokens 및 POST /api/v1/tokens/{token_id}/revoke를 사용합니다. revoke는 같은 ID에 반복 호출해도 성공하는 자연 멱등 연산이며 별도 Idempotency-Key가 필요하지 않습니다. 다른 프로젝트 ID는 404, 미허용 역할은 403, 철회된 토큰은 401입니다.

업그레이드: 서버를 중단하고 백업 후 migrate를 실행합니다. schema v3는 기존 토큰의 종전 읽기/쓰기/검토 기능을 보존하려고 reviewer로 이관합니다. 기존 토큰이 자동으로 admin으로 승격되지는 않습니다. 관리자용 토큰은 DB CLI에서 grant --role admin으로 명시 발급하세요. 새 프로젝트의 provision 토큰만 기본 admin입니다. 역할 변경/회전은 새 역할의 토큰 발급 후 구 토큰 철회로 수행합니다.

팀별 권한, API 발급, 만료 시각, 감사 원장, 전체 조직 정책은 아직 구현되지 않았습니다.

멘션 팀 등록·조회는 프로젝트 관리자 권한입니다. 수신함과 확인 상태는 해당 수신자 principal의 토큰에만 노출됩니다. [멘션 안내](mentions.md)를 참고하세요.
