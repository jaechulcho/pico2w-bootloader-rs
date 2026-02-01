# pico2w-bootloader-rs (Pico 2 W Rust Bootloader)

Raspberry Pi Pico 2 W (RP2350)를 위한 안전하고 견고한 Rust 기반 부트로더입니다.

## 주요 기능

- **무결성 검증 (CRC32)**: 애플리케이션의 매직 넘버("APPS")와 전체 이미지의 CRC32 체크섬을 검증하여 안전한 부팅을 보장합니다.
- **자동 DFU 모드 진입**: 애플리케이션이 없거나 체크섬이 손상된 경우 자동으로 업데이트 모드로 진입합니다.
- **UART DFU (Device Firmware Update)**: 시리얼 통신(UART)을 통해 새로운 펌웨어를 업로드할 수 있습니다.
- **안전한 부팅 (Safe Jump)**: `VTOR` 설정 및 스택 포인터 검증을 통해 애플리케이션으로 안전하게 점프합니다.

## 메모리 레이아웃

- **Bootloader (64KB)**: `0x10000000` ~ `0x1000FFFF`
- **Metadata (256B)**: `0x10010000` ~ `0x100100FF` (Magic, Length, CRC32 포함)
- **Application**: `0x10010100` ~

## UART DFU 프로토콜

업데이트 모드에서 사용되는 프로토콜 사양:
- **Baudrate**: 115200 bps
1. **시작 시그널**: `'u'` 키 (3초 이내 입력 시 수동 진입)
2. **매직 바이트**: `0xAA`
3. **헤더 (8 bytes)**: `[Length (4B, LE) | CRC32 (4B, LE)]`
4. **데이터**: 애플리케이션 바이너리 (`Length` 만큼 전송)

## 빌드 및 실행

```bash
# 빌드
cargo build --release

# 실행 및 모니터링
cargo run --release
```

## 연동 가이드

애플리케이션(예: `pico2w-shell-rs`)은 반드시 이 부트로더의 메모리 맵(`0x10010100`)에 맞춰 링크되어야 하며, `package_app.py`와 같은 툴을 이용해 메타데이터 헤더가 포함된 상태로 플래싱되어야 합니다.
