---
name: ponytail
description: >-
  Find the smallest maintainable implementation by reusing existing code,
  platform features, or installed dependencies. Use for coding, refactoring,
  and code review when the user asks for Ponytail, YAGNI, fewer dependencies,
  or help reducing overengineering.
license: MIT
---

# Ponytail

Find the smallest readable change that fully solves the task and follows the
applicable project and user instructions.

## Modes

Use `$ponytail` followed by a task. Specify `lite`, `full`, or `ultra` after
`$ponytail` to choose a mode. Default to `full`.

- **Lite.** Complete the requested approach. Mention a materially simpler option
  briefly if one exists.
- **Full.** Use the implementation order below to choose the simplest complete
  approach within the request.
- **Ultra.** Also flag requirements that add substantial complexity for little
  benefit. Explain the tradeoff and respect the user's choice.

Keep an explicitly selected mode for subsequent coding tasks in the current
conversation. If the user says `ponytail off` or `stop ponytail`, stop applying
the skill in that conversation until explicitly re-enabled.

## Implementation order

1. Keep the requested behavior and omit speculative features, options, and
   extension points.
2. Reuse a suitable project helper, component, service, or established pattern.
3. Use a standard library or platform feature if the project lacks a suitable
   solution.
4. Reuse an installed dependency when it fits. Add a new dependency only when
   its concrete benefits justify the maintenance cost.
5. Write the minimum readable code needed for the remaining work. Avoid
   single-use wrappers, needless conversions, and dense one-liners.

Check callers before changing shared behavior. Keep the validation, data loss
protection, security safeguards, accessibility, and hardware calibration the task
requires. Explain material limits of shortcuts when they affect the result.

During review, flag concrete opportunities to reuse or simplify code and explain
which behavior each suggestion preserves.

## Sources

Adapted from [Ponytail](https://github.com/DietrichGebert/ponytail/blob/main/skills/ponytail/SKILL.md).
The baseline coding guidelines in the companion global `AGENTS.md` draw on
[karpathy-guidelines](https://github.com/multica-ai/andrej-karpathy-skills/blob/main/skills/karpathy-guidelines/SKILL.md).
See [LICENSE](LICENSE) for attribution and the MIT notice.
