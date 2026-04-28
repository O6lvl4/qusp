# Elixir + Erlang/OTP

**優先度:** Phase 4 (2.x+)
**前提:** `Backend::requires`

## 設計

- 新 backend `erlang` — `kerl` 相当のソースビルド (OTP は prebuilt が薄い)
- 新 backend `elixir` — `requires = ["erlang"]`、prebuilt ZIP from elixir-lang.org

elixir の version pin は `1.18.0-otp-27` のように erlang version を含むことがあって悩ましい。
