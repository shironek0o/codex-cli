---
name: inkdrop
description: Draft an implementation plan and save it to a selected Inkdrop note. Use when the user invokes $inkdrop or explicitly asks to plan a coding task in Inkdrop.
---

# Plan in Inkdrop

Use the Inkdrop MCP tools. If they are unavailable, point the user to the [Inkdrop MCP setup](https://docs.inkdrop.app/reference/mcp-server#codex) and stop before saving.

## Select the destination

- If the invocation includes an `inkdrop://note/<id>` link or a `note:<id>`, read that note. Use `includeTemplate: true` to load its planning template when present. A supplied note link or ID identifies the destination without a picker.
- If the user provides a title or search term, search Inkdrop for matching notes. If the user provides no destination, ask whether to find an existing note or create a new one. For an existing note, ask for a title or search term, or let the user choose a notebook and list its notes. Show a short numbered list with note titles and, when needed to distinguish them, notebook names. Wait for the user to select a match. Do not choose by recency or title alone.
- For a new note, list notebooks and let the user select one, unless they have already named a unique notebook. Reuse an explicit title or derive one from the task. Create the note only when the plan is ready to save.

## Plan and save

Use the task supplied in the invocation or the requirements in the selected note. Ask for the task if neither is available. Inspect the relevant code, then draft implementation steps and verification checks. Ask about decisions that could materially change the approach.

For a note created from a planning template, fill its plan sections while retaining the user's requirements and notes. For another existing note, append an implementation-plan section and preserve the original text. Use the current note revision for updates. For a new note, save the plan as its body.

Return the saved note's `inkdrop://note/<id>` link and wait for the user to confirm before implementing any changes. If the save fails, report the error without claiming it succeeded.
