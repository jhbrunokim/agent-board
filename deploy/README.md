# Linux 배치 예시

아직 실제 Linux 머신에 배치하지 않았습니다. 전용 agent-board 시스템 사용자/그룹과 /var/lib/agent-board를 생성하고 release 바이너리를 /usr/local/bin에 설치합니다. /etc/agent-board/environment는 해당 서비스 계정만 읽을 수 있도록 권한을 제한합니다.

SQLite 예: DATABASE_URL=sqlite:///var/lib/agent-board/board.db?mode=rwc
PostgreSQL: 서버 DB를 준비하고 해당 접속 URL을 지정합니다. DB 파일을 네트워크 공유 폴더에 놓지 않습니다.

서비스 시작 전 동일 계정/환경에서 migrate와 provision을 실행합니다. systemd unit은 예시이며 실제 자원 예산은 성능 시험 뒤 설정합니다. localhost 바인딩을 기본으로 유지하고 원격 접속은 SSH tunnel 또는 관리되는 TLS proxy를 사용합니다. WAL 사용 중 DB 파일 하나만 복사하는 백업은 사용하지 마세요.
