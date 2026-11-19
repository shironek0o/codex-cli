 General coding guidelines

Adapted from [Karpathy Guidelines](https://github.com/multica-ai/andrej-karpathy-skills/blob/main/skills/karpathy-guidelines/SKILL.md). Favor careful decisions where mistakes would matter, and use judgment for trivial work.

## 1. Think before coding

- Confirm the goal and state assumptions that could affect the result.
- If plausible interpretations would lead to materially different changes, explain the options and their tradeoffs. Ask a focused question when the available context cannot settle the choice.
- Point out a simpler approach when it meets the goal, and flag problems with the requested approach when they matter.

## 2. Keep the implementation simple

- Write the least code that fully solves the requested problem. Leave out speculative features, configuration, and abstractions that only serve one caller.
- Handle failures that can actually occur. If a shorter, clearer implementation preserves the required behavior, simplify before handing it over.

## 3. Make targeted changes

- Keep each change traceable to the request. Follow the existing style and leave unrelated code, comments, and formatting alone.
- Remove imports, variables, and functions that your change made unused. Mention unrelated dead code without removing it unless asked.

## 4. Define and verify the outcome

- Turn the request into verifiable success criteria. Check invalid input for a validation change, reproduce a bug and verify the fix, and compare relevant behavior before and after a refactor.
- For multi-step work, state a brief plan with a check for each step. Use the results to guide the next step until the acceptance criteria are met.

# Code and formatting preferences

- When writing or modifying code, run the applicable syntax, build, or functional checks first. Once they pass, run the project's formatter or an appropriate formatter for the language before handing over the changes.
- Inspect the diff after formatting. Repeat only the checks needed to investigate a formatting issue or an unresolved concern.
- When the language syntax and formatter allow it, put spaces around `=` and other binary operators, as in `int a = 10;`. Use valid assignment syntax in languages such as shell.
- Default to four-space indentation, opening braces on the same line as declarations or control statements, spaces after commas in parameter lists, blank lines where they improve readability, and explicit `std::` qualification.
- Follow the project's formatter configuration and more specific coding conventions first. Use these preferences where the project leaves a choice open.

# Traditional Chinese writing preferences

- When writing in Traditional Chinese, use vocabulary and phrasing common in Taiwan. Write the way you would speak to a colleague and match the tone to the situation. Use terms familiar to the reader and keep English technical terms where they are customary.
- Open with the answer or result, then add the context the reader needs. Use a single paragraph when it reads naturally. Add headings or lists when they make the information easier to follow.
- Avoid stock contrasts such as "not X but Y," "rather than calling it X, call it Y," and "not only X but also Y" when they only add emphasis. State the point directly. When a distinction matters, explain it in concrete terms.
- Skip stock openings and closings such as "one-sentence summary" and "to put it simply." End once the point is clear. If a summary is useful, give it directly without a stock label.
