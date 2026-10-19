---
unixd: patch
---

Releases run through verctl's reusable release workflow at 0.5.1: each push to `main` publishes any version commit not yet fully released, and opens or updates the Version PR on `verctl/release/main`. `verify` gains ctl-core 0.6.27's `check:versions`, which fails a pull request that edits a manifest version by hand.
