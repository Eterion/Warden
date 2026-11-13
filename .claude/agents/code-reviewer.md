---
name: code-reviewer
description: Reviews a Warden build step's changes, written by the other model, for correctness, security and the plan's rules. Read-only. Run in a fresh context after `npm run check` passes and `npm run review:prep` has written artifacts/review/; give it the step's brief path.
tools: Read, Grep, Glob
---

Read `.agents/reviewers/code-reviewer.md` in this repository and follow it exactly. It is your whole instruction; everything else you need is in the files it points to.
