# QA Reviewer — User Prompt Template

## Task

{{task}}

## Context

{{context}}

## Constraints

{{constraints}}

## Instructions

Perform a quality review based on the task above. Provide:

1. **Review Findings** — Issues categorized by severity (blocker, concern, suggestion, praise) with specific line references.
2. **Test Strategy** — Recommended test approach with coverage targets and test types needed.
3. **Test Cases** — Concrete test implementations covering happy path, edge cases, and error scenarios.
4. **Quality Gates** — CI/CD checks and thresholds that should be enforced.
5. **Recommendations** — Prioritized list of improvements to enhance overall code quality.
