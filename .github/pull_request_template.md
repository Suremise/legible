## What and why

<!-- What this changes, and the problem it solves. Link the issue: "Fixes #12". -->

## How to check it

<!-- What a reviewer can run or try to see it working. For a change to what's on screen, before and after screenshots. -->

## Checklist

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace` (or `cargo test --workspace`) and `cargo deny check` pass
- [ ] For changes in `web/`: `node scripts/doc-tests.mjs` and `node scripts/chrome-tests.mjs` pass
- [ ] New dependencies pass the licence rule (MIT or equally permissive, no copyleft, system fonts only: see [CONTRIBUTING.md](https://github.com/Suremise/legible/blob/main/CONTRIBUTING.md#dependencies-the-licence-rule)), or there are none
- [ ] Tests use temporary folders only, wait for conditions rather than time, and a fix has a test that fails without it
- [ ] New user-visible text is in `locales/en/`, not in the code
- [ ] Docs updated where they're affected (`docs/user-guide.md`, `docs/ARCHITECTURE.md`), and an `Unreleased` entry in `CHANGELOG.md` for anything a user would notice
