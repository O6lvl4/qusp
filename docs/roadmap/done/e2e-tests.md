# e2e Test Scenarios

**Shipped:** scripts/e2e/* (commit 0f7e00a 周辺)

各 backend に install → run → tool routing → cleanup の e2e テスト。
- `scripts/e2e/{go,ruby,python,node,deno,bun,java,rust,kotlin,smoke}.sh`
- `scripts/e2e.sh` ドライバ (--fast, 個別指定 OK)
- `scripts/e2e/common.sh` で isolated $HOME / assert_* / step / ok/fail/skip
- `.github/workflows/e2e.yml` で nightly cron + workflow_dispatch (ubuntu + macos-14)

publisher 側の breakage (PBS の version dropping, Foojay schema 変更等) を
nightly で先回り検知する。
