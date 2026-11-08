# 간단 설치

## 현재 지원: 소스 설치

비공개 저장소 접근 권한과 GitHub CLI 로그인, Rust stable, 플랫폼 C 빌드 도구가 필요합니다.

```sh
gh repo clone jhbrunokim/agent-board
cd agent-board
./scripts/install.sh
```

설치 위치는 기본 `~/.local/bin/agent-board`입니다. `./scripts/install.sh --prefix /absolute/path`로 변경합니다. 스크립트는 release 빌드 후 실행 파일만 교체하며 DB·토큰·서비스를 자동 생성하거나 변경하지 않습니다. PATH에 설치 디렉터리를 추가한 뒤 `agent-board --help`를 실행합니다. 같은 절차를 다시 실행하면 바이너리를 업그레이드합니다. 기존 DB 백업과 `agent-board migrate`는 별도 단계입니다.

이미 빌드된 실행 파일은 `./scripts/install.sh --binary /absolute/path/agent-board`로 설치할 수 있습니다. 설치 파일은 해당 OS/CPU에 맞는 신뢰된 파일을 사용합니다. 아직 공식 prebuilt release는 발행하지 않았습니다.

## 목표: Rust 없이 설치

버전 태그 기반 GitHub Release에서 Linux x86_64/ARM64, macOS Apple Silicon/Intel 실행 파일과 SHA-256 manifest를 제공합니다. 다운로드 설치기는 OS/CPU를 판별하고 버전 및 checksum을 검증한 뒤 같은 설치 로직을 사용하게 합니다. Linux glibc 요구 버전 또는 musl 지원을 release마다 명시하고 실제 대상에서 시험합니다. private release는 gh 인증으로 다운로드하며 token을 명령줄/로그에 출력하지 않습니다. 검증 없는 curl-pipe-shell을 필수 설치 방식으로 삼지 않습니다.

이 단계는 release 빌드/실기기 검증 후 제공합니다. 현재 내려받을 수 있는 URL이나 지원 완료 플랫폼을 가정하지 않습니다.

## Homebrew 방향

전용 `jhbrunokim/homebrew-tap`에 formula를 두고 동일한 버전별 release archive/checksum을 사용합니다. 목표 명령은 다음과 같습니다.

```sh
brew tap jhbrunokim/tap
brew install agent-board
```

아직 tap/formula를 발행하지 않았으므로 위 명령은 현재 설치 경로가 아닙니다. private tap과 private release의 인증이 일반 Homebrew 다운로드에 자동 적용된다고 가정하지 않습니다. 배포 시 공개 바이너리/tap을 별도로 제공할지, 인증된 전용 설치기를 유지할지 결정합니다. 소스 저장소 공개는 설치 편의를 위해 자동 변경하지 않습니다.

Linux 서버의 systemd 서비스 등록은 `deploy/` 예시로 분리하고, 서비스 계정·데이터 위치·접근 범위를 확인한 뒤 실행합니다. CLI 설치와 운영 서비스 등록을 구분합니다.
