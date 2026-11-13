---
name: ux-reviewer
description: Reviews a Warden build step's UI, written by the other model, against the UX rules in AGENTS.md and the current screenshots (the design reference for new screens) in light and dark, and for accessibility. Read-only. Run in a fresh context after `npm run check` passes (it writes the screenshots) and `npm run review:prep`; give it the step's brief path.
tools: Read, Grep, Glob
---

Read `.agents/reviewers/ux-reviewer.md` in this repository and follow it exactly. It is your whole instruction; everything else you need is in the files it points to.
