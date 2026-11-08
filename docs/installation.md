# 설치와 업그레이드

## GitHub 릴리스로 설치

GitHub CLI에 로그인하고 비공개 저장소 접근 권한을 확인합니다. 쓰기 가능한 디렉터리에서 설치 파일을 받습니다.

```sh
gh release download --repo jhbrunokim/agent-board --pattern install.sh
bash install.sh --release latest
```

지원 빌드는 Linux x86-64·ARM64와 macOS Apple Silicon·Intel입니다. Linux는 Ubuntu 22.04, macOS는 macOS 15에서 빌드하고 실행을 확인합니다. 다른 배포판이나 OS 버전에서 문제가 있으면 소스 설치를 사용하세요.

설치기는 GitHub CLI로 플랫폼에 맞는 실행 파일과 SHA-256 목록을 받고, 체크섬과 버전 태그를 확인한 뒤 설치합니다. Bash와 `sha256sum` 또는 `shasum`이 필요합니다. 기본 위치는 `~/.local/bin/agent-board`이며 `--prefix /absolute/path`로 지정할 수 있습니다.

특정 버전을 쓰려면 다운로드와 설치에 같은 태그를 지정합니다.

```sh
gh release download v0.1.0 --repo jhbrunokim/agent-board --pattern install.sh
bash install.sh --release v0.1.0
```

## 소스에서 설치

Rust stable과 플랫폼의 C 빌드 도구가 필요합니다.

```sh
gh repo clone jhbrunokim/agent-board
cd agent-board
./scripts/install.sh
```

신뢰할 수 있는 사전 빌드 파일을 이미 받았다면 `./scripts/install.sh --binary /absolute/path/agent-board`를 사용합니다.

## 서버 업그레이드

서비스를 중단하고 DB를 백업한 뒤 새 바이너리를 설치합니다. 기존 DB 접속 설정으로 `agent-board migrate`를 실행하고 서비스를 다시 시작합니다. [Linux 운영 안내](../deploy/README.md)를 참고하세요.

CLI 설치와 서버 배치는 별도입니다. 현재 간편 설치 경로는 비공개 GitHub 릴리스와 로그인한 GitHub CLI를 사용합니다. Homebrew tap을 배포하려면 비공개 바이너리를 받는 인증 경로를 별도로 마련해야 합니다.

## 릴리스 운영

Cargo.toml과 Cargo.lock에서 버전을 준비하고 main의 계약 테스트를 확인합니다. 아래 워크플로는 네 플랫폼의 빌드, 버전 확인, 설치, SQLite 스키마 생성이 모두 성공한 뒤 해당 커밋의 릴리스를 생성합니다.

```sh
gh workflow run release.yml --ref main -f tag=v0.1.0 -f publish=true
```

`publish=false`는 빌드와 검증만 실행합니다. 실패한 실행에서 초안이 남았다면 태그, 커밋과 업로드된 자산을 확인한 뒤 복구하세요.
